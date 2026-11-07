//! Offsets of the target's game memory (64-bit, ASLR enabled).
//!
//! * `g_game` at `module base + RVA_G_GAME` holds a pointer to `Game`.
//! * `Game` is `{ Fighter *hero; Fighter *monster; int round; }`.
//! * `Fighter` is `{ char name[16]; int hp; int max_hp; int min_dmg; int max_dmg; int defending; }`.

/// RVA of the global `g_game` pointer, relative to the module base.
pub const RVA_G_GAME: usize = 0x100A0;
/// Offset of the hero `Fighter *` inside `Game`.
pub const OFF_HERO: usize = 0x00;
/// Offset of `hp` inside `Fighter`.
pub const F_HP: usize = 0x10;
/// Offset of `max_hp` inside `Fighter`.
pub const F_MAX_HP: usize = 0x14;
