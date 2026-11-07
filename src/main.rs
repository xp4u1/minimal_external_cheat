//! Minimal external cheat for `game.exe`.
//!
//! Attaches to the running game, sets the hero's HP and max HP once, and then keeps the hero
//! at full health (god mode) until the game exits.
//!
//! * [`process`] attaches to the game and reads/writes its memory.
//! * [`offsets`] holds the game's memory layout.

#[cfg(not(target_pointer_width = "64"))]
compile_error!("the target game is 64-bit; build this cheat for a 64-bit target");

mod offsets;
mod process;

use std::io;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use offsets::{F_HP, F_MAX_HP, OFF_HERO, RVA_G_GAME};
use process::Process;

/// Executable name of the target process.
const PROC_NAME: &str = "game.exe";

/// HP and max HP applied once when the hero is first seen.
const HERO_HP: i32 = 99_999;

/// Delay between two god-mode refreshes.
const TICK: Duration = Duration::from_millis(20);
/// Delay between attach attempts while waiting for the game.
const POLL: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Waits for the game, then applies the cheat until the game exits.
fn run() -> io::Result<()> {
    println!("Waiting for {PROC_NAME} ...");
    let process = loop {
        if let Some(process) = Process::try_attach(PROC_NAME)? {
            break process;
        }
        thread::sleep(POLL);
    };
    println!("Attached, module base {:#x}", process.base);

    let mut hp_set = false;
    loop {
        if process.exited() {
            println!("Game exited.");
            return Ok(());
        }

        if let Err(err) = tick(&process, &mut hp_set) {
            // A failure caused by the game closing is not an error; the check above ends the loop.
            if !process.exited() {
                return Err(err);
            }
        }

        thread::sleep(TICK);
    }
}

/// One refresh: sets the initial HP on first sight of the hero, then restores full health.
///
/// Does nothing while the game has not created the hero yet.
fn tick(process: &Process, hp_set: &mut bool) -> io::Result<()> {
    let game = process.read_ptr(process.base + RVA_G_GAME)?;
    if game == 0 {
        return Ok(());
    }
    let hero = process.read_ptr(game + OFF_HERO)?;
    if hero == 0 {
        return Ok(());
    }

    if !*hp_set {
        process.write_i32(hero + F_MAX_HP, HERO_HP)?;
        process.write_i32(hero + F_HP, HERO_HP)?;
        println!("Hero HP set to {HERO_HP}, god mode on.");
        *hp_set = true;
    }

    let max_hp = process.read_i32(hero + F_MAX_HP)?;
    process.write_i32(hero + F_HP, max_hp)
}
