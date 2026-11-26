//! The WebAssembly exports (WEB-R3): plain functions over numbers, so the page needs no
//! generated glue, only `site/static/player.js`.
//!
//! Strings cross the boundary through one byte buffer in this module's memory: the page
//! asks for `input(len)`, writes UTF-8 there and calls the function that reads it; output
//! strings are left in `output` and read back through `output_ptr`/`output_len`. Players
//! live in a table and are named by a handle (0 means none). Frame pixels stay inside the
//! player; `frame` returns their address in linear memory.
//!
//! The exports need `#[unsafe(no_mangle)]`, the only `unsafe` in the crate: no pointer is
//! ever dereferenced here, addresses are only handed to the page as numbers.
#![allow(unsafe_code)]

use std::cell::RefCell;

use sciart_render::Theme;

use crate::{Player, scenes, themes};

thread_local! {
    static PLAYERS: RefCell<Vec<Option<Player>>> = const { RefCell::new(Vec::new()) };
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn with_player<T>(handle: u32, default: T, f: impl FnOnce(&mut Player) -> T) -> T {
    PLAYERS.with_borrow_mut(|players| {
        match handle
            .checked_sub(1)
            .and_then(|i| players.get_mut(i as usize))
        {
            Some(Some(player)) => f(player),
            _ => default,
        }
    })
}

fn set_output(s: String) {
    OUTPUT.with_borrow_mut(|out| *out = s);
}

/// A buffer of `len` bytes for the page to write a string into; returns its address.
#[unsafe(no_mangle)]
pub extern "C" fn input(len: u32) -> u32 {
    INPUT.with_borrow_mut(|buf| {
        buf.clear();
        buf.resize(len as usize, 0);
        buf.as_ptr() as u32
    })
}

/// The address and length of the last string an export produced.
#[unsafe(no_mangle)]
pub extern "C" fn output_ptr() -> u32 {
    OUTPUT.with_borrow(|out| out.as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn output_len() -> u32 {
    OUTPUT.with_borrow(|out| out.len() as u32)
}

/// Leave the scene catalogue (`sciart list --json`) in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn scenes_json() {
    set_output(scenes());
}

/// Leave the theme names, one per line, in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn theme_names() {
    set_output(themes().join("\n"));
}

/// A player for the scene named in the input buffer; 0 when there is no such scene. The
/// seed arrives as two 32-bit halves, since JavaScript numbers hold 53 bits.
#[unsafe(no_mangle)]
pub extern "C" fn player_new(columns: u32, rows: u32, seed_hi: u32, seed_lo: u32) -> u32 {
    let name = INPUT.with_borrow(|buf| String::from_utf8_lossy(buf).into_owned());
    let seed = (u64::from(seed_hi) << 32) | u64::from(seed_lo);
    let Some(player) = Player::new(&name, columns, rows, seed) else {
        return 0;
    };
    PLAYERS.with_borrow_mut(|players| {
        let slot = players.iter().position(Option::is_none).unwrap_or_else(|| {
            players.push(None);
            players.len() - 1
        });
        players[slot] = Some(player);
        slot as u32 + 1
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn player_free(handle: u32) {
    PLAYERS.with_borrow_mut(|players| {
        if let Some(slot) = handle
            .checked_sub(1)
            .and_then(|i| players.get_mut(i as usize))
        {
            *slot = None;
        }
    });
}

/// `Player::advance`: 1 when the time was taken, 0 when it was refused.
#[unsafe(no_mangle)]
pub extern "C" fn player_advance(handle: u32, seconds: f64) -> u32 {
    with_player(handle, 0, |p| u32::from(p.advance(seconds)))
}

#[unsafe(no_mangle)]
pub extern "C" fn player_resize(handle: u32, columns: u32, rows: u32) {
    with_player(handle, (), |p| p.resize(columns, rows));
}

/// Draw the frame in theme number `theme` (the order of `theme_names`); returns the address
/// of `player_width × player_height × 4` RGBA bytes, valid until the next call into the
/// module.
#[unsafe(no_mangle)]
pub extern "C" fn player_frame(handle: u32, theme: u32) -> u32 {
    let theme = Theme::ALL
        .get(theme as usize)
        .copied()
        .unwrap_or(Theme::Astro);
    with_player(handle, 0, |p| p.frame(theme).as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn player_width(handle: u32) -> u32 {
    with_player(handle, 0, |p| p.width())
}

#[unsafe(no_mangle)]
pub extern "C" fn player_height(handle: u32) -> u32 {
    with_player(handle, 0, |p| p.height())
}

/// The registered name of the player's scene, left in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn player_scene(handle: u32) {
    let name = with_player(handle, "", |p| p.scene());
    set_output(name.to_owned());
}
