use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
    inode_id: u32,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
        inode_id: u32,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
            inode_id
        }
    }

    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }

    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }

    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.name() == name {
                return Some(dirent.inode_number());
            }
        }
        None
    }

    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        // 目录查找流程：目录 inode -> 遍历 dirent -> 定位子 inode 的磁盘位置。
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                    inode_id,
                ))
            })
        })
    }

    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        // 先按“新增块数”批量申请数据块，再一次性扩容 inode。
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }

    fn clear_locked(&self, fs: &mut MutexGuard<EasyFileSystem>) {
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
    }

    /// Create inode under current inode by name.
    /// Attention: use find previously to ensure the new file not existing.
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        // 1) 分配新 inode
        let new_inode_id = fs.alloc_inode();
        // 2) 初始化 inode 元数据
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        // 3) 在当前目录追加 dirent 项
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // 4) 返回新文件的 Inode 句柄
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
            new_inode_id,
        )))
        // release efs lock automatically by compiler
    }

    /// 获取 inode 的元数据。
    pub fn stat(&self) -> (u32, bool, u32) {
        self.read_disk_inode(|disk_inode| (self.inode_id, disk_inode.is_dir(), disk_inode.nlink))
    }

    /// 在当前目录下为 inode 创建一个硬链接。
    pub fn create_hard_link(&self, name: &str, inode: &Arc<Inode>) -> isize {
        if inode.read_disk_inode(|disk_inode| disk_inode.is_dir()) {
            return -1;
        }

        let mut fs = self.fs.lock();
        let target_inode_id = inode.inode_id;
        let inserted = self.modify_disk_inode(|root_inode| {
            if !root_inode.is_dir() || self.find_inode_id(name, root_inode).is_some() {
                return false;
            }

            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            self.increase_size(new_size as u32, root_inode, &mut fs);
            let dirent = DirEntry::new(name, target_inode_id);
            assert_eq!(
                root_inode.write_at(file_count * DIRENT_SZ, dirent.as_bytes(), &self.block_device),
                DIRENT_SZ,
            );
            true
        });
        if !inserted {
            return -1;
        }

        let (block_id, block_offset) = fs.get_disk_inode_pos(target_inode_id);
        get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |disk_inode: &mut DiskInode| {
                disk_inode.nlink += 1;
            });
        block_cache_sync_all();
        0
    }

    /// 从当前目录下删除一个目录项。
    ///
    /// 当该目录项是目标 inode 的最后一个硬链接时，同时回收 inode 和数据块。
    pub fn unlink_child(&self, name: &str) -> isize {
        let mut fs = self.fs.lock();
        let target_inode_id = self.modify_disk_inode(|root_inode| {
            if !root_inode.is_dir() {
                return None;
            }

            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let mut dirent = DirEntry::empty();
            let mut target_index = None;
            for i in 0..file_count {
                assert_eq!(
                    root_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device),
                    DIRENT_SZ,
                );
                if dirent.name() == name {
                    target_index = Some(i);
                    break;
                }
            }

            let target_index = target_index?;
            let last_index = file_count.checked_sub(1)?;
            if target_index != last_index {
                let mut last = DirEntry::empty();
                assert_eq!(
                    root_inode.read_at(last_index * DIRENT_SZ, last.as_bytes_mut(), &self.block_device),
                    DIRENT_SZ,
                );
                assert_eq!(
                    root_inode.write_at(target_index * DIRENT_SZ, last.as_bytes(), &self.block_device),
                    DIRENT_SZ,
                );
            }
            root_inode.size -= DIRENT_SZ as u32;
            Some(dirent.inode_number())
        });
        let Some(target_inode_id) = target_inode_id else {
            return -1;
        };

        let (block_id, block_offset) = fs.get_disk_inode_pos(target_inode_id);
        let remaining_links = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |disk_inode: &mut DiskInode| {
                disk_inode.nlink -= 1;
                disk_inode.nlink
            });
        if remaining_links == 0 {
            let target = Self::new(
                block_id,
                block_offset,
                self.fs.clone(),
                self.block_device.clone(),
                target_inode_id,
            );
            target.clear_locked(&mut fs);
            fs.dealloc_inode(target_inode_id);
        }

        block_cache_sync_all();
        0
    }

    /// List inodes by id under current inode
    pub fn readdir(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }

    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }

    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }

    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.clear_locked(&mut fs);
        block_cache_sync_all();
    }

    /// Search for a file or directory by name under the current inode
    pub fn search(&self, name: &str) -> Option<Arc<Inode>> {
        self.find(name)
    }
}
