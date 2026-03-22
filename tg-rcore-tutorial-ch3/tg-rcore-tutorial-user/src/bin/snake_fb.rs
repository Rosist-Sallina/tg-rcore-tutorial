#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
extern crate user_lib;

use alloc::boxed::Box;
use user_lib::{fb_fill_rect, fb_info, fb_present, get_time, input_try_getchar, sleep};

const BOARD_SIZE: usize = 20;
const BOARD_CAPACITY: usize = BOARD_SIZE * BOARD_SIZE;
const CELL_SIZE: usize = 10;
const BOARD_PIXELS: usize = BOARD_SIZE * CELL_SIZE;
const BORDER: usize = 2;

const COLOR_BG: u32 = rgb(0x00, 0x00, 0x00);
const COLOR_BOARD: u32 = rgb(0xff, 0xff, 0xff);
const COLOR_BORDER: u32 = rgb(0xff, 0xd6, 0x00);
const COLOR_HEAD: u32 = rgb(0xff, 0x00, 0x00);
const COLOR_BODY: u32 = rgb(0x00, 0xff, 0x00);
const COLOR_FOOD: u32 = rgb(0xff, 0x00, 0xff);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Point {
    x: u8,
    y: u8,
}

impl Point {
    const ZERO: Self = Self { x: 0, y: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn is_opposite(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Up, Self::Down)
                | (Self::Down, Self::Up)
                | (Self::Left, Self::Right)
                | (Self::Right, Self::Left)
        )
    }
}

struct SnakeGame {
    body: [Point; BOARD_CAPACITY],
    len: usize,
    dir: Direction,
    food: Point,
    alive: bool,
    score: usize,
    seed: u32,
}

impl SnakeGame {
    fn new(seed: u32) -> Self {
        let mut game = Self {
            body: [Point::ZERO; BOARD_CAPACITY],
            len: 0,
            dir: Direction::Right,
            food: Point::ZERO,
            alive: true,
            score: 0,
            seed,
        };
        game.reset(seed);
        game
    }

    fn reset(&mut self, seed: u32) {
        self.body = [Point::ZERO; BOARD_CAPACITY];
        self.len = 3;
        self.dir = Direction::Right;
        self.alive = true;
        self.score = 0;
        self.seed = seed.wrapping_add(0x9e37_79b9);

        let center = BOARD_SIZE / 2;
        self.body[0] = Point {
            x: center as u8,
            y: center as u8,
        };
        self.body[1] = Point {
            x: (center - 1) as u8,
            y: center as u8,
        };
        self.body[2] = Point {
            x: (center - 2) as u8,
            y: center as u8,
        };
        self.place_food();
    }

    fn advance(&mut self) {
        if !self.alive {
            return;
        }

        let mut next = self.body[0];
        match self.dir {
            Direction::Up => {
                if next.y == 0 {
                    self.alive = false;
                    return;
                }
                next.y -= 1;
            }
            Direction::Down => {
                next.y += 1;
                if next.y as usize >= BOARD_SIZE {
                    self.alive = false;
                    return;
                }
            }
            Direction::Left => {
                if next.x == 0 {
                    self.alive = false;
                    return;
                }
                next.x -= 1;
            }
            Direction::Right => {
                next.x += 1;
                if next.x as usize >= BOARD_SIZE {
                    self.alive = false;
                    return;
                }
            }
        }

        let grow = next == self.food;
        let collision_len = if grow {
            self.len
        } else {
            self.len.saturating_sub(1)
        };
        if self.body[..collision_len].contains(&next) {
            self.alive = false;
            return;
        }

        let new_len = if grow {
            self.len + 1
        } else {
            self.len
        };
        for idx in (1..new_len).rev() {
            self.body[idx] = self.body[idx - 1];
        }
        self.body[0] = next;
        self.len = new_len;

        if grow {
            self.score += 1;
            println!("score = {}", self.score);
            if self.len == BOARD_CAPACITY {
                self.alive = false;
            } else {
                self.place_food();
            }
        }
    }

    fn place_food(&mut self) {
        let start = self.random(BOARD_CAPACITY as u32) as usize;
        for offset in 0..BOARD_CAPACITY {
            let idx = (start + offset) % BOARD_CAPACITY;
            let candidate = Point {
                x: (idx % BOARD_SIZE) as u8,
                y: (idx / BOARD_SIZE) as u8,
            };
            if !self.body[..self.len].contains(&candidate) {
                self.food = candidate;
                return;
            }
        }
        self.alive = false;
    }

    fn random(&mut self, limit: u32) -> u32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.seed % limit
    }
}

enum InputAction {
    None,
    Quit,
    Restart,
    Turn(Direction),
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
    let width = info.width as usize;
    let height = info.height as usize;
    let board_left = width.saturating_sub(BOARD_PIXELS) / 2;
    let board_top = height.saturating_sub(BOARD_PIXELS) / 2;

    println!("snake_fb framebuffer = {}x{}", info.width, info.height);
    println!("controls: w/a/s/d move, q quit, r restart after death");

    let seed = get_time() as u32;
    let game_local = SnakeGame::new(seed);
    let mut game = Box::new(game_local);
    render(&game, width, height, board_left, board_top);

    loop {
        if game.alive {
            match poll_input(game.dir, true) {
                InputAction::Turn(dir) => game.dir = dir,
                InputAction::Quit => return 0,
                InputAction::None | InputAction::Restart => {}
            }
            sleep(150);
            game.advance();
            render(&game, width, height, board_left, board_top);
            if !game.alive {
                println!("game over, score = {}", game.score);
                println!("press r to restart, q to quit");
            }
        } else {
            match poll_input(game.dir, false) {
                InputAction::Restart => {
                    game.reset((get_time() as u32) ^ game.seed);
                    println!("restart");
                    render(&game, width, height, board_left, board_top);
                }
                InputAction::Quit => return 0,
                InputAction::None | InputAction::Turn(_) => sleep(50),
            }
        }
    }
}

fn poll_input(current: Direction, alive: bool) -> InputAction {
    let mut last_dir = None;
    while let Some(ch) = input_try_getchar() {
        match ch {
            b'w' | b'W' => update_dir(&mut last_dir, current, Direction::Up),
            b's' | b'S' => update_dir(&mut last_dir, current, Direction::Down),
            b'a' | b'A' => update_dir(&mut last_dir, current, Direction::Left),
            b'd' | b'D' => update_dir(&mut last_dir, current, Direction::Right),
            b'q' | b'Q' => return InputAction::Quit,
            b'r' | b'R' if !alive => return InputAction::Restart,
            _ => {}
        }
    }
    last_dir.map_or(InputAction::None, InputAction::Turn)
}

fn update_dir(slot: &mut Option<Direction>, current: Direction, next: Direction) {
    let active = slot.unwrap_or(current);
    if !active.is_opposite(next) {
        *slot = Some(next);
    }
}

fn render(game: &SnakeGame, width: usize, height: usize, board_left: usize, board_top: usize) {
    let _ = fb_fill_rect(0, 0, width, height, COLOR_BG);
    let _ = fb_fill_rect(board_left, board_top, BOARD_PIXELS, BOARD_PIXELS, COLOR_BOARD);

    let outer_left = board_left.saturating_sub(BORDER);
    let outer_top = board_top.saturating_sub(BORDER);
    let outer_w = BOARD_PIXELS + BORDER * 2;
    let outer_h = BOARD_PIXELS + BORDER * 2;
    let _ = fb_fill_rect(outer_left, outer_top, outer_w, BORDER, COLOR_BORDER);
    let _ = fb_fill_rect(
        outer_left,
        outer_top + outer_h - BORDER,
        outer_w,
        BORDER,
        COLOR_BORDER,
    );
    let _ = fb_fill_rect(outer_left, outer_top, BORDER, outer_h, COLOR_BORDER);
    let _ = fb_fill_rect(
        outer_left + outer_w - BORDER,
        outer_top,
        BORDER,
        outer_h,
        COLOR_BORDER,
    );

    for idx in 0..game.len {
        let point = game.body[idx];
        let color = if idx == 0 { COLOR_HEAD } else { COLOR_BODY };
        draw_cell(point, board_left, board_top, color);
    }
    draw_cell(game.food, board_left, board_top, COLOR_FOOD);
    let _ = fb_present();
}

fn draw_cell(point: Point, board_left: usize, board_top: usize, color: u32) {
    let pixel_y = board_top + point.y as usize * CELL_SIZE;
    let pixel_x = board_left + point.x as usize * CELL_SIZE;
    let _ = fb_fill_rect(pixel_x, pixel_y, CELL_SIZE, CELL_SIZE, color);
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}
