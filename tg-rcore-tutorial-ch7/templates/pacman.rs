#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
extern crate user_lib;

use alloc::boxed::Box;
use core::{
    cmp::{max, min},
    ptr::null,
    sync::atomic::{AtomicBool, Ordering},
};
use user_lib::{
    close, fb_fill_rect, fb_info, fb_present, fork, get_time, getpid, input_try_getchar, kill,
    pipe, pipe_write, read, sigaction, sigreturn, sleep, waitpid, SignalAction, SignalNo,
};

const MAP_W: usize = 21;
const MAP_H: usize = 21;
const MAP_CELLS: usize = MAP_W * MAP_H;
const GHOST_COUNT: usize = 2;
const HUD_HEIGHT: usize = 28;
const BOARD_MARGIN: usize = 12;
const TICK_MS: usize = 120;
const RESPAWN_TICKS: u8 = 6;

const TILE_WALL: u8 = 0;
const TILE_DOT: u8 = 1;
const TILE_EMPTY: u8 = 2;

const COLOR_BG: u32 = rgb(0x04, 0x08, 0x14);
const COLOR_HUD: u32 = rgb(0x10, 0x1c, 0x30);
const COLOR_BOARD: u32 = rgb(0x08, 0x10, 0x1f);
const COLOR_BORDER: u32 = rgb(0x3d, 0x6b, 0xff);
const COLOR_WALL: u32 = rgb(0x1e, 0x52, 0xff);
const COLOR_DOT: u32 = rgb(0xff, 0xf0, 0x99);
const COLOR_PACMAN: u32 = rgb(0xff, 0xd6, 0x00);
const COLOR_GHOST_0: u32 = rgb(0xff, 0x4d, 0x6d);
const COLOR_GHOST_1: u32 = rgb(0x4d, 0xe0, 0xff);
const COLOR_EYE: u32 = rgb(0xff, 0xff, 0xff);
const COLOR_PAUSED: u32 = rgb(0xe9, 0xe3, 0xd5);
const COLOR_LIFE_LOST: u32 = rgb(0xff, 0x9f, 0x1c);
const COLOR_VICTORY: u32 = rgb(0x2e, 0xcc, 0x71);
const COLOR_GAME_OVER: u32 = rgb(0xe6, 0x39, 0x46);

static PAUSE_TOGGLE: AtomicBool = AtomicBool::new(false);
static RESTART_REQUEST: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Point {
    x: u8,
    y: u8,
}

impl Point {
    const fn new(x: usize, y: usize) -> Self {
        Self {
            x: x as u8,
            y: y as u8,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Left,
    Down,
    Right,
}

impl Direction {
    const ALL: [Self; 4] = [Self::Up, Self::Left, Self::Down, Self::Right];

    fn delta(self) -> (isize, isize) {
        match self {
            Self::Up => (0, -1),
            Self::Left => (-1, 0),
            Self::Down => (0, 1),
            Self::Right => (1, 0),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Left => Self::Right,
            Self::Down => Self::Up,
            Self::Right => Self::Left,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Paused,
    LifeLost,
    Victory,
    GameOver,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GhostKind {
    Hunter,
    Wanderer,
}

#[derive(Clone, Copy)]
struct Ghost {
    pos: Point,
    dir: Direction,
    kind: GhostKind,
}

#[derive(Clone, Copy)]
struct Layout {
    width: usize,
    height: usize,
    cell: usize,
    board_left: usize,
    board_top: usize,
    board_width: usize,
    board_height: usize,
}

impl Layout {
    fn new(width: usize, height: usize) -> Self {
        let cell_w = width.saturating_sub(BOARD_MARGIN * 2) / MAP_W;
        let cell_h = height.saturating_sub(HUD_HEIGHT + BOARD_MARGIN * 2) / MAP_H;
        let cell = max(6, min(cell_w, cell_h));
        let board_width = cell * MAP_W;
        let board_height = cell * MAP_H;
        let board_left = width.saturating_sub(board_width) / 2;
        let board_top =
            HUD_HEIGHT + height.saturating_sub(HUD_HEIGHT + board_height + BOARD_MARGIN) / 2;
        Self {
            width,
            height,
            cell,
            board_left,
            board_top,
            board_width,
            board_height,
        }
    }
}

#[derive(Clone, Copy)]
struct WallRect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

const PACMAN_START: Point = Point::new(2, 2);
const GHOST_STARTS: [Point; GHOST_COUNT] = [Point::new(9, 10), Point::new(11, 10)];
const WALL_RECTS: [WallRect; 9] = [
    WallRect {
        x: 3,
        y: 3,
        w: 3,
        h: 3,
    },
    WallRect {
        x: 8,
        y: 3,
        w: 5,
        h: 2,
    },
    WallRect {
        x: 15,
        y: 3,
        w: 3,
        h: 3,
    },
    WallRect {
        x: 5,
        y: 8,
        w: 2,
        h: 5,
    },
    WallRect {
        x: 9,
        y: 8,
        w: 3,
        h: 2,
    },
    WallRect {
        x: 14,
        y: 8,
        w: 2,
        h: 5,
    },
    WallRect {
        x: 3,
        y: 15,
        w: 3,
        h: 3,
    },
    WallRect {
        x: 8,
        y: 16,
        w: 5,
        h: 2,
    },
    WallRect {
        x: 15,
        y: 15,
        w: 3,
        h: 3,
    },
];

struct Game {
    tiles: [u8; MAP_CELLS],
    pacman: Point,
    current_dir: Direction,
    wanted_dir: Direction,
    ghosts: [Ghost; GHOST_COUNT],
    remaining_dots: usize,
    score: usize,
    lives: u8,
    phase: Phase,
    respawn_ticks: u8,
    seed: u32,
}

impl Game {
    fn new(seed: u32) -> Self {
        let mut game = Self {
            tiles: [TILE_EMPTY; MAP_CELLS],
            pacman: PACMAN_START,
            current_dir: Direction::Right,
            wanted_dir: Direction::Right,
            ghosts: [
                Ghost {
                    pos: GHOST_STARTS[0],
                    dir: Direction::Left,
                    kind: GhostKind::Hunter,
                },
                Ghost {
                    pos: GHOST_STARTS[1],
                    dir: Direction::Right,
                    kind: GhostKind::Wanderer,
                },
            ],
            remaining_dots: 0,
            score: 0,
            lives: 3,
            phase: Phase::Running,
            respawn_ticks: 0,
            seed,
        };
        game.reset(seed);
        game
    }

    fn reset(&mut self, seed: u32) {
        self.seed = seed.wrapping_add(0x9e37_79b9);
        self.score = 0;
        self.lives = 3;
        self.phase = Phase::Running;
        self.respawn_ticks = 0;
        self.reset_board();
        self.reset_positions();
    }

    fn reset_board(&mut self) {
        self.tiles.fill(TILE_DOT);
        for x in 0..MAP_W {
            self.set_tile(Point::new(x, 0), TILE_WALL);
            self.set_tile(Point::new(x, MAP_H - 1), TILE_WALL);
        }
        for y in 0..MAP_H {
            self.set_tile(Point::new(0, y), TILE_WALL);
            self.set_tile(Point::new(MAP_W - 1, y), TILE_WALL);
        }
        for rect in WALL_RECTS {
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    self.set_tile(Point::new(x, y), TILE_WALL);
                }
            }
        }
        self.set_tile(PACMAN_START, TILE_EMPTY);
        for point in GHOST_STARTS {
            self.set_tile(point, TILE_EMPTY);
        }
        self.remaining_dots = self
            .tiles
            .iter()
            .filter(|tile| **tile == TILE_DOT)
            .count();
    }

    fn reset_positions(&mut self) {
        self.pacman = PACMAN_START;
        self.current_dir = Direction::Right;
        self.wanted_dir = Direction::Right;
        self.ghosts = [
            Ghost {
                pos: GHOST_STARTS[0],
                dir: Direction::Left,
                kind: GhostKind::Hunter,
            },
            Ghost {
                pos: GHOST_STARTS[1],
                dir: Direction::Right,
                kind: GhostKind::Wanderer,
            },
        ];
        self.respawn_ticks = 0;
    }

    fn tick(&mut self) {
        match self.phase {
            Phase::Running => self.tick_running(),
            Phase::LifeLost => {
                if self.respawn_ticks > 0 {
                    self.respawn_ticks -= 1;
                }
                if self.respawn_ticks == 0 {
                    self.reset_positions();
                    self.phase = Phase::Running;
                }
            }
            Phase::Paused | Phase::Victory | Phase::GameOver => {}
        }
    }

    fn tick_running(&mut self) {
        if let Some(next) = self.next_point(self.pacman, self.wanted_dir) {
            self.current_dir = self.wanted_dir;
            self.pacman = next;
        } else if let Some(next) = self.next_point(self.pacman, self.current_dir) {
            self.pacman = next;
        }

        self.consume_dot();
        if self.remaining_dots == 0 {
            self.phase = Phase::Victory;
            println!("victory! score = {}", self.score);
            return;
        }
        if self.check_collision() {
            return;
        }

        for i in 0..GHOST_COUNT {
            let ghost = self.ghosts[i];
            let dir = match ghost.kind {
                GhostKind::Hunter => self.choose_hunter_dir(ghost),
                GhostKind::Wanderer => self.choose_wander_dir(ghost),
            };
            if let Some(next) = self.next_point(ghost.pos, dir) {
                self.ghosts[i].pos = next;
                self.ghosts[i].dir = dir;
            }
        }
        self.check_collision();
    }

    fn consume_dot(&mut self) {
        if self.tile_at(self.pacman) == TILE_DOT {
            self.set_tile(self.pacman, TILE_EMPTY);
            self.remaining_dots -= 1;
            self.score += 10;
        }
    }

    fn choose_hunter_dir(&self, ghost: Ghost) -> Direction {
        let (dirs, len) = self.collect_dirs(ghost.pos, ghost.dir);
        if len == 0 {
            return ghost.dir;
        }
        let mut best = dirs[0];
        let mut best_dist = self.distance_after_move(ghost.pos, dirs[0]);
        for dir in dirs.iter().copied().take(len).skip(1) {
            let dist = self.distance_after_move(ghost.pos, dir);
            if dist < best_dist || (dist == best_dist && dir == ghost.dir) {
                best = dir;
                best_dist = dist;
            }
        }
        best
    }

    fn choose_wander_dir(&mut self, ghost: Ghost) -> Direction {
        let (dirs, len) = self.collect_dirs(ghost.pos, ghost.dir);
        if len == 0 {
            return ghost.dir;
        }
        dirs[self.random(len as u32) as usize]
    }

    fn collect_dirs(&self, point: Point, current: Direction) -> ([Direction; 4], usize) {
        let mut dirs = [Direction::Up; 4];
        let mut len = 0;
        for dir in Direction::ALL {
            if self.next_point(point, dir).is_some() {
                dirs[len] = dir;
                len += 1;
            }
        }
        if len <= 1 {
            return (dirs, len);
        }
        let opposite = current.opposite();
        let mut filtered = [Direction::Up; 4];
        let mut filtered_len = 0;
        for dir in dirs.iter().copied().take(len) {
            if dir != opposite {
                filtered[filtered_len] = dir;
                filtered_len += 1;
            }
        }
        if filtered_len == 0 {
            (dirs, len)
        } else {
            (filtered, filtered_len)
        }
    }

    fn distance_after_move(&self, point: Point, dir: Direction) -> usize {
        if let Some(next) = self.next_point(point, dir) {
            self.manhattan(next, self.pacman)
        } else {
            usize::MAX
        }
    }

    fn manhattan(&self, a: Point, b: Point) -> usize {
        a.x.abs_diff(b.x) as usize + a.y.abs_diff(b.y) as usize
    }

    fn next_point(&self, point: Point, dir: Direction) -> Option<Point> {
        let (dx, dy) = dir.delta();
        let next_x = point.x as isize + dx;
        let next_y = point.y as isize + dy;
        if next_x < 0 || next_y < 0 {
            return None;
        }
        let next = Point::new(next_x as usize, next_y as usize);
        (self.tile_at(next) != TILE_WALL).then_some(next)
    }

    fn check_collision(&mut self) -> bool {
        if self.ghosts.iter().any(|ghost| ghost.pos == self.pacman) {
            self.lose_life();
            true
        } else {
            false
        }
    }

    fn lose_life(&mut self) {
        if self.lives > 1 {
            self.lives -= 1;
            self.phase = Phase::LifeLost;
            self.respawn_ticks = RESPAWN_TICKS;
            println!("life lost, remaining lives = {}", self.lives);
        } else {
            self.lives = 0;
            self.phase = Phase::GameOver;
            println!("game over, score = {}", self.score);
        }
    }

    fn toggle_pause(&mut self) {
        match self.phase {
            Phase::Running => {
                self.phase = Phase::Paused;
                println!("paused");
            }
            Phase::Paused => {
                self.phase = Phase::Running;
                println!("resume");
            }
            Phase::LifeLost | Phase::Victory | Phase::GameOver => {}
        }
    }

    fn restart_if_possible(&mut self, seed: u32) {
        if matches!(self.phase, Phase::Victory | Phase::GameOver) {
            self.reset(seed);
            println!("restart");
        }
    }

    fn frame_delay(&self) -> usize {
        match self.phase {
            Phase::Running => TICK_MS,
            Phase::LifeLost => 100,
            Phase::Paused => 60,
            Phase::Victory | Phase::GameOver => 80,
        }
    }

    fn random(&mut self, limit: u32) -> u32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.seed % limit
    }

    fn tile_at(&self, point: Point) -> u8 {
        self.tiles[self.index(point)]
    }

    fn set_tile(&mut self, point: Point, tile: u8) {
        let idx = self.index(point);
        self.tiles[idx] = tile;
    }

    fn index(&self, point: Point) -> usize {
        point.y as usize * MAP_W + point.x as usize
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    let info = match fb_info() {
        Some(info) => info,
        None => {
            println!("fb_info failed");
            return 1;
        }
    };
    let layout = Layout::new(info.width as usize, info.height as usize);
    println!("pacman framebuffer = {}x{}", info.width, info.height);
    println!("controls: w/a/s/d move, p pause, r restart after end, q quit");

    if install_signal_handlers() < 0 {
        println!("sigaction failed");
        return 1;
    }

    let mut pipe_fd = [0usize; 2];
    if pipe(&mut pipe_fd) < 0 {
        println!("pipe failed");
        return 1;
    }

    let parent_pid = getpid();
    let child_pid = fork();
    if child_pid < 0 {
        println!("fork failed");
        let _ = close(pipe_fd[0]);
        let _ = close(pipe_fd[1]);
        return 1;
    }
    if child_pid == 0 {
        let _ = close(pipe_fd[0]);
        return input_process(pipe_fd[1], parent_pid);
    }

    let _ = close(pipe_fd[1]);
    let mut game = Box::new(Game::new(get_time() as u32));
    render(&game, layout);

    loop {
        if drain_pipe(pipe_fd[0], &mut game) {
            break;
        }
        if PAUSE_TOGGLE.swap(false, Ordering::Relaxed) {
            game.toggle_pause();
        }
        if RESTART_REQUEST.swap(false, Ordering::Relaxed) {
            game.restart_if_possible((get_time() as u32) ^ game.seed);
        }
        game.tick();
        render(&game, layout);
        sleep(game.frame_delay());
    }

    let _ = close(pipe_fd[0]);
    let mut exit_code = 0;
    let _ = waitpid(child_pid, &mut exit_code);
    0
}

fn install_signal_handlers() -> isize {
    let pause = SignalAction {
        handler: pause_handler as usize,
        mask: 0,
    };
    let restart = SignalAction {
        handler: restart_handler as usize,
        mask: 0,
    };
    let ret1 = sigaction(SignalNo::SIGUSR1, &pause, null::<SignalAction>());
    let ret2 = sigaction(SignalNo::SIGUSR2, &restart, null::<SignalAction>());
    if ret1 < 0 || ret2 < 0 {
        -1
    } else {
        0
    }
}

fn pause_handler() {
    PAUSE_TOGGLE.store(true, Ordering::Relaxed);
    sigreturn();
}

fn restart_handler() {
    RESTART_REQUEST.store(true, Ordering::Relaxed);
    sigreturn();
}

fn input_process(write_fd: usize, parent_pid: isize) -> i32 {
    loop {
        let mut touched = false;
        while let Some(ch) = input_try_getchar() {
            touched = true;
            match ch {
                b'w' | b'W' => {
                    let _ = pipe_write(write_fd, b"w");
                }
                b'a' | b'A' => {
                    let _ = pipe_write(write_fd, b"a");
                }
                b's' | b'S' => {
                    let _ = pipe_write(write_fd, b"s");
                }
                b'd' | b'D' => {
                    let _ = pipe_write(write_fd, b"d");
                }
                b'p' | b'P' => {
                    let _ = kill(parent_pid, SignalNo::SIGUSR1);
                }
                b'r' | b'R' => {
                    let _ = kill(parent_pid, SignalNo::SIGUSR2);
                }
                b'q' | b'Q' => {
                    let _ = pipe_write(write_fd, b"q");
                    let _ = close(write_fd);
                    return 0;
                }
                _ => {}
            }
        }
        if !touched {
            sleep(10);
        }
    }
}

fn drain_pipe(read_fd: usize, game: &mut Game) -> bool {
    let mut buf = [0u8; 1];
    loop {
        let ret = read(read_fd, &mut buf);
        if ret == -2 {
            return false;
        }
        if ret == 0 {
            return true;
        }
        if ret < 0 {
            return false;
        }
        match buf[0] {
            b'w' => game.wanted_dir = Direction::Up,
            b'a' => game.wanted_dir = Direction::Left,
            b's' => game.wanted_dir = Direction::Down,
            b'd' => game.wanted_dir = Direction::Right,
            b'q' => return true,
            _ => {}
        }
    }
}

fn render(game: &Game, layout: Layout) {
    draw_rect(0, 0, layout.width, layout.height, COLOR_BG);
    draw_rect(0, 0, layout.width, HUD_HEIGHT, COLOR_HUD);
    draw_rect(
        layout.board_left.saturating_sub(2),
        layout.board_top.saturating_sub(2),
        layout.board_width + 4,
        layout.board_height + 4,
        COLOR_BORDER,
    );
    draw_rect(
        layout.board_left,
        layout.board_top,
        layout.board_width,
        layout.board_height,
        COLOR_BOARD,
    );

    for y in 0..MAP_H {
        for x in 0..MAP_W {
            let point = Point::new(x, y);
            match game.tile_at(point) {
                TILE_WALL => draw_cell(point, layout, COLOR_WALL),
                TILE_DOT => draw_dot(point, layout),
                _ => {}
            }
        }
    }

    draw_pacman(game.pacman, game.current_dir, layout);
    for (idx, ghost) in game.ghosts.iter().enumerate() {
        let color = if idx == 0 {
            COLOR_GHOST_0
        } else {
            COLOR_GHOST_1
        };
        draw_ghost(ghost.pos, layout, color);
    }

    draw_lives(game.lives as usize, layout);
    draw_phase_overlay(game.phase, layout);
    let _ = fb_present();
}

fn draw_lives(lives: usize, layout: Layout) {
    let icon = max(6, layout.cell / 2);
    let y = max(4, (HUD_HEIGHT.saturating_sub(icon)) / 2);
    for i in 0..lives {
        let x = 8 + i * (icon + 6);
        draw_rect(x, y, icon, icon, COLOR_PACMAN);
        let mouth = max(2, icon / 3);
        draw_rect(x + icon / 2, y + icon / 3, mouth, mouth, COLOR_HUD);
    }
}

fn draw_phase_overlay(phase: Phase, layout: Layout) {
    let color = match phase {
        Phase::Running => return,
        Phase::Paused => COLOR_PAUSED,
        Phase::LifeLost => COLOR_LIFE_LOST,
        Phase::Victory => COLOR_VICTORY,
        Phase::GameOver => COLOR_GAME_OVER,
    };
    let banner_w = max(layout.cell * 6, layout.board_width / 3);
    let banner_h = max(layout.cell, 10);
    let x = layout.board_left + layout.board_width.saturating_sub(banner_w) / 2;
    let y = layout.board_top + layout.board_height.saturating_sub(banner_h) / 2;
    draw_rect(x, y, banner_w, banner_h, color);
}

fn draw_cell(point: Point, layout: Layout, color: u32) {
    let (x, y) = cell_origin(point, layout);
    draw_rect(x, y, layout.cell, layout.cell, color);
}

fn draw_dot(point: Point, layout: Layout) {
    let (x, y) = cell_origin(point, layout);
    let dot = max(2, layout.cell / 4);
    let offset = (layout.cell.saturating_sub(dot)) / 2;
    draw_rect(x + offset, y + offset, dot, dot, COLOR_DOT);
}

fn draw_pacman(point: Point, dir: Direction, layout: Layout) {
    let (x, y) = cell_origin(point, layout);
    let inset = max(1, layout.cell / 8);
    let size = layout.cell.saturating_sub(inset * 2);
    let body_x = x + inset;
    let body_y = y + inset;
    draw_rect(body_x, body_y, size, size, COLOR_PACMAN);
    let cut = max(2, size / 3);
    match dir {
        Direction::Up => draw_rect(body_x + cut, body_y, cut, size / 2, COLOR_BOARD),
        Direction::Left => draw_rect(body_x, body_y + cut, size / 2, cut, COLOR_BOARD),
        Direction::Down => draw_rect(body_x + cut, body_y + size / 2, cut, size / 2, COLOR_BOARD),
        Direction::Right => draw_rect(body_x + size / 2, body_y + cut, size / 2, cut, COLOR_BOARD),
    }
}

fn draw_ghost(point: Point, layout: Layout, color: u32) {
    let (x, y) = cell_origin(point, layout);
    let inset = max(1, layout.cell / 8);
    let size = layout.cell.saturating_sub(inset * 2);
    let body_x = x + inset;
    let body_y = y + inset;
    draw_rect(body_x, body_y, size, size, color);
    let eye = max(1, size / 6);
    if size >= 8 {
        draw_rect(body_x + size / 4, body_y + size / 4, eye, eye, COLOR_EYE);
        draw_rect(body_x + size / 2, body_y + size / 4, eye, eye, COLOR_EYE);
    }
}

fn cell_origin(point: Point, layout: Layout) -> (usize, usize) {
    (
        layout.board_left + point.x as usize * layout.cell,
        layout.board_top + point.y as usize * layout.cell,
    )
}

fn draw_rect(x: usize, y: usize, w: usize, h: usize, color: u32) {
    let _ = fb_fill_rect(x, y, w, h, color);
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}
