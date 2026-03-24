#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{fb_fill_rect, fb_info, fb_present, input_try_getchar, sleep};
use user_lib::{fork, mailbox_create, mailbox_recv, mailbox_send, waitpid};

const BORDER: usize = 4;
const PADDLE_W: usize = 6;
const PADDLE_H: usize = 40;
const BALL_SIZE: usize = 6;
const PADDLE_SPEED: isize = 6;
const BALL_SPEED_X: isize = 4;
const BALL_SPEED_Y: isize = 3;
const TARGET_SCORE: u8 = 7;
const SCORE_W: usize = 18;
const SCORE_H: usize = 28;
const SCORE_SEG: usize = 4;

const COLOR_BG: u32 = rgb(0x08, 0x10, 0x14);
const COLOR_FIELD: u32 = rgb(0x10, 0x1f, 0x23);
const COLOR_BORDER: u32 = rgb(0xff, 0xd6, 0x00);
const COLOR_LEFT: u32 = rgb(0x2e, 0xcc, 0x71);
const COLOR_RIGHT: u32 = rgb(0x39, 0x82, 0xff);
const COLOR_BALL: u32 = rgb(0xff, 0xf5, 0xf1);
const COLOR_SCORE: u32 = rgb(0xf8, 0x4f, 0x39);

#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

struct Game {
    width: usize,
    height: usize,
    arena_left: usize,
    arena_top: usize,
    arena_right: usize,
    arena_bottom: usize,
    left_score: u8,
    right_score: u8,
    left_y: isize,
    right_y: isize,
    ball_x: isize,
    ball_y: isize,
    ball_vx: isize,
    ball_vy: isize,
    finished: bool,
    winner_left: bool,
}

impl Game {
    fn new(width: usize, height: usize) -> Self {
        let arena_left = 16;
        let arena_top = 44;
        let arena_right = width.saturating_sub(16);
        let arena_bottom = height.saturating_sub(16);
        let mut game = Self {
            width,
            height,
            arena_left,
            arena_top,
            arena_right,
            arena_bottom,
            left_score: 0,
            right_score: 0,
            left_y: 0,
            right_y: 0,
            ball_x: 0,
            ball_y: 0,
            ball_vx: BALL_SPEED_X,
            ball_vy: BALL_SPEED_Y,
            finished: false,
            winner_left: false,
        };
        game.reset_match();
        game
    }

    fn reset_match(&mut self) {
        self.left_score = 0;
        self.right_score = 0;
        self.finished = false;
        self.reset_round(true);
    }

    fn reset_round(&mut self, serve_left: bool) {
        let center_y = ((self.arena_top + self.arena_bottom).saturating_sub(PADDLE_H)) / 2;
        self.left_y = center_y as isize;
        self.right_y = center_y as isize;
        self.ball_x = ((self.arena_left + self.arena_right).saturating_sub(BALL_SIZE)) as isize / 2;
        self.ball_y = ((self.arena_top + self.arena_bottom).saturating_sub(BALL_SIZE)) as isize / 2;
        self.ball_vx = if serve_left {
            BALL_SPEED_X
        } else {
            -BALL_SPEED_X
        };
        self.ball_vy = BALL_SPEED_Y;
    }

    fn left_paddle_x(&self) -> usize {
        self.arena_left + 10
    }

    fn right_paddle_x(&self) -> usize {
        self.arena_right.saturating_sub(10 + PADDLE_W)
    }

    fn paddle_top_limit(&self) -> isize {
        self.arena_top as isize
    }

    fn paddle_bottom_limit(&self) -> isize {
        self.arena_bottom.saturating_sub(PADDLE_H) as isize
    }

    fn move_left(&mut self, delta: isize) {
        self.left_y = (self.left_y + delta).clamp(self.paddle_top_limit(), self.paddle_bottom_limit());
    }

    fn move_right(&mut self, delta: isize) {
        self.right_y = (self.right_y + delta).clamp(self.paddle_top_limit(), self.paddle_bottom_limit());
    }

    fn tick(&mut self) {
        if self.finished {
            return;
        }

        self.ball_x += self.ball_vx;
        self.ball_y += self.ball_vy;

        if self.ball_y <= self.arena_top as isize {
            self.ball_y = self.arena_top as isize;
            self.ball_vy = self.ball_vy.abs();
        }
        let bottom_limit = self.arena_bottom.saturating_sub(BALL_SIZE) as isize;
        if self.ball_y >= bottom_limit {
            self.ball_y = bottom_limit;
            self.ball_vy = -self.ball_vy.abs();
        }

        let left_x = self.left_paddle_x() as isize;
        let right_x = self.right_paddle_x() as isize;
        if self.ball_vx < 0
            && self.ball_x <= left_x + PADDLE_W as isize
            && self.ball_x + BALL_SIZE as isize >= left_x
            && self.overlap_with_paddle(self.left_y)
        {
            self.ball_x = left_x + PADDLE_W as isize;
            self.ball_vx = self.ball_vx.abs();
            self.adjust_ball_vy(self.left_y);
        }
        if self.ball_vx > 0
            && self.ball_x + BALL_SIZE as isize >= right_x
            && self.ball_x <= right_x + PADDLE_W as isize
            && self.overlap_with_paddle(self.right_y)
        {
            self.ball_x = right_x - BALL_SIZE as isize;
            self.ball_vx = -self.ball_vx.abs();
            self.adjust_ball_vy(self.right_y);
        }

        if self.ball_x + (BALL_SIZE as isize) < self.arena_left as isize {
            self.right_score += 1;
            println!("right score = {}", self.right_score);
            self.finish_or_reset(false);
        } else if self.ball_x > self.arena_right as isize {
            self.left_score += 1;
            println!("left score = {}", self.left_score);
            self.finish_or_reset(true);
        }
    }

    fn overlap_with_paddle(&self, paddle_y: isize) -> bool {
        let ball_top = self.ball_y;
        let ball_bottom = self.ball_y + BALL_SIZE as isize;
        let paddle_top = paddle_y;
        let paddle_bottom = paddle_y + PADDLE_H as isize;
        ball_bottom >= paddle_top && ball_top <= paddle_bottom
    }

    fn adjust_ball_vy(&mut self, paddle_y: isize) {
        let ball_center = self.ball_y + BALL_SIZE as isize / 2;
        let paddle_center = paddle_y + PADDLE_H as isize / 2;
        let relative = ball_center - paddle_center;
        self.ball_vy = match relative {
            ..=-14 => -5,
            -13..=-5 => -3,
            -4..=4 => {
                if self.ball_vy >= 0 {
                    2
                } else {
                    -2
                }
            }
            5..=13 => 3,
            _ => 5,
        };
    }

    fn finish_or_reset(&mut self, winner_left: bool) {
        if self.left_score >= TARGET_SCORE || self.right_score >= TARGET_SCORE {
            self.finished = true;
            self.winner_left = winner_left;
            println!(
                "{} player wins! press r to restart, q to quit",
                if winner_left { "left" } else { "right" }
            );
        } else {
            self.reset_round(winner_left);
        }
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
    println!("pong framebuffer = {}x{}", info.width, info.height);
    println!("controls: w/s left paddle, o/l right paddle, r restart, q quit");
    draw_startup_pattern(info.width as usize, info.height as usize);
    let _ = fb_present();
    sleep(1000);

    let mut game = Game::new(info.width as usize, info.height as usize);
    let mailbox = mailbox_create();
    if mailbox < 0 {
        println!("mailbox_create failed");
        return 1;
    }
    let child_pid = fork();
    if child_pid < 0 {
        println!("fork failed");
        return 1;
    }
    if child_pid == 0 {
        return input_process(mailbox as usize);
    }

    render(&game);

    loop {
        if drain_mailbox(mailbox as usize, &mut game) {
            break;
        }
        if !game.finished {
            sleep(20);
            game.tick();
            render(&game);
        } else {
            sleep(40);
        }
    }

    let mut exit_code = 0;
    let _ = waitpid(child_pid, &mut exit_code);
    0
}

fn input_process(mailbox: usize) -> i32 {
    loop {
        let mut sent = false;
        while let Some(ch) = input_try_getchar() {
            if mailbox_send(mailbox, ch as usize) == 0 {
                sent = true;
            }
            if matches!(ch, b'q' | b'Q') {
                return 0;
            }
        }
        if !sent {
            sleep(10);
        }
    }
}

fn drain_mailbox(mailbox: usize, game: &mut Game) -> bool {
    loop {
        let event = mailbox_recv(mailbox);
        if event < 0 {
            return false;
        }
        match event as u8 {
            b'w' | b'W' => game.move_left(-PADDLE_SPEED),
            b's' | b'S' => game.move_left(PADDLE_SPEED),
            b'o' | b'O' => game.move_right(-PADDLE_SPEED),
            b'l' | b'L' => game.move_right(PADDLE_SPEED),
            b'r' | b'R' => game.reset_match(),
            b'q' | b'Q' => return true,
            _ => {}
        }
    }
}

fn render(game: &Game) {
    let _ = fb_fill_rect(0, 0, game.width, game.height, COLOR_BG);
    let _ = fb_fill_rect(
        game.arena_left,
        game.arena_top,
        game.arena_right.saturating_sub(game.arena_left),
        game.arena_bottom.saturating_sub(game.arena_top),
        COLOR_FIELD,
    );
    draw_border(game);
    draw_center_line(game);
    draw_scores(game);
    draw_rect(
        Rect {
            x: game.left_paddle_x(),
            y: game.left_y as usize,
            w: PADDLE_W,
            h: PADDLE_H,
        },
        COLOR_LEFT,
    );
    draw_rect(
        Rect {
            x: game.right_paddle_x(),
            y: game.right_y as usize,
            w: PADDLE_W,
            h: PADDLE_H,
        },
        COLOR_RIGHT,
    );
    if !game.finished {
        draw_rect(
            Rect {
                x: game.ball_x as usize,
                y: game.ball_y as usize,
                w: BALL_SIZE,
                h: BALL_SIZE,
            },
            COLOR_BALL,
        );
    } else {
        let marker_x = if game.winner_left {
            game.left_paddle_x() + 20
        } else {
            game.right_paddle_x().saturating_sub(20)
        };
        draw_rect(
            Rect {
                x: marker_x,
                y: game.arena_top + 20,
                w: 8,
                h: game.arena_bottom.saturating_sub(game.arena_top + 40),
            },
            COLOR_BALL,
        );
    }
    let _ = fb_present();
}

fn draw_startup_pattern(width: usize, height: usize) {
    let third = width / 3;
    let _ = fb_fill_rect(0, 0, third, height, rgb(0xff, 0x20, 0x20));
    let _ = fb_fill_rect(third, 0, third, height, rgb(0x20, 0xff, 0x20));
    let _ = fb_fill_rect(third * 2, 0, width.saturating_sub(third * 2), height, rgb(0x20, 0x40, 0xff));
    let _ = fb_fill_rect(width / 2 - 10, height / 2 - 10, 20, 20, rgb(0xff, 0xff, 0xff));
}

fn draw_border(game: &Game) {
    draw_rect(
        Rect {
            x: game.arena_left.saturating_sub(BORDER),
            y: game.arena_top.saturating_sub(BORDER),
            w: game.arena_right.saturating_sub(game.arena_left) + BORDER * 2,
            h: BORDER,
        },
        COLOR_BORDER,
    );
    draw_rect(
        Rect {
            x: game.arena_left.saturating_sub(BORDER),
            y: game.arena_bottom,
            w: game.arena_right.saturating_sub(game.arena_left) + BORDER * 2,
            h: BORDER,
        },
        COLOR_BORDER,
    );
    draw_rect(
        Rect {
            x: game.arena_left.saturating_sub(BORDER),
            y: game.arena_top.saturating_sub(BORDER),
            w: BORDER,
            h: game.arena_bottom.saturating_sub(game.arena_top) + BORDER * 2,
        },
        COLOR_BORDER,
    );
    draw_rect(
        Rect {
            x: game.arena_right,
            y: game.arena_top.saturating_sub(BORDER),
            w: BORDER,
            h: game.arena_bottom.saturating_sub(game.arena_top) + BORDER * 2,
        },
        COLOR_BORDER,
    );
}

fn draw_center_line(game: &Game) {
    let center_x = (game.arena_left + game.arena_right) / 2;
    let mut y = game.arena_top + 8;
    while y + 8 < game.arena_bottom {
        draw_rect(
            Rect {
                x: center_x.saturating_sub(1),
                y,
                w: 2,
                h: 10,
            },
            COLOR_BORDER,
        );
        y += 18;
    }
}

fn draw_scores(game: &Game) {
    let center_x = game.width / 2;
    let score_y = 10;
    draw_digit(center_x.saturating_sub(40), score_y, game.left_score);
    draw_digit(center_x + 22, score_y, game.right_score);
}

fn draw_digit(x: usize, y: usize, digit: u8) {
    let mask = match digit {
        0 => 0b1111110,
        1 => 0b0110000,
        2 => 0b1101101,
        3 => 0b1111001,
        4 => 0b0110011,
        5 => 0b1011011,
        6 => 0b1011111,
        7 => 0b1110000,
        8 => 0b1111111,
        _ => 0b1111011,
    };
    let seg = [
        Rect {
            x: x + SCORE_SEG,
            y,
            w: SCORE_W.saturating_sub(SCORE_SEG * 2),
            h: SCORE_SEG,
        },
        Rect {
            x,
            y: y + SCORE_SEG,
            w: SCORE_SEG,
            h: SCORE_H / 2 - SCORE_SEG,
        },
        Rect {
            x: x + SCORE_W.saturating_sub(SCORE_SEG),
            y: y + SCORE_SEG,
            w: SCORE_SEG,
            h: SCORE_H / 2 - SCORE_SEG,
        },
        Rect {
            x: x + SCORE_SEG,
            y: y + SCORE_H / 2 - SCORE_SEG / 2,
            w: SCORE_W.saturating_sub(SCORE_SEG * 2),
            h: SCORE_SEG,
        },
        Rect {
            x,
            y: y + SCORE_H / 2,
            w: SCORE_SEG,
            h: SCORE_H / 2 - SCORE_SEG,
        },
        Rect {
            x: x + SCORE_W.saturating_sub(SCORE_SEG),
            y: y + SCORE_H / 2,
            w: SCORE_SEG,
            h: SCORE_H / 2 - SCORE_SEG,
        },
        Rect {
            x: x + SCORE_SEG,
            y: y + SCORE_H.saturating_sub(SCORE_SEG),
            w: SCORE_W.saturating_sub(SCORE_SEG * 2),
            h: SCORE_SEG,
        },
    ];
    for (idx, rect) in seg.iter().enumerate() {
        if mask & (1 << (6 - idx)) != 0 {
            draw_rect(*rect, COLOR_SCORE);
        }
    }
}

fn draw_rect(rect: Rect, color: u32) {
    let _ = fb_fill_rect(rect.x, rect.y, rect.w, rect.h, color);
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}
