use core::ffi::c_char;

unsafe extern "C" {
    fn doomgeneric_Create(argc: i32, argv: *const *const c_char);
    fn doomgeneric_Tick();
}

pub(crate) fn run() -> i32 {
    let argv = [b"doom\0".as_ptr() as *const c_char];
    unsafe {
        doomgeneric_Create(1, argv.as_ptr());
        loop {
            doomgeneric_Tick();
        }
    }
}
