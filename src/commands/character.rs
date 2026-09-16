use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::commands::{human_bytes, unix_to_date};
use crate::format::fch::{CharacterFile, stat_name};
use crate::names::NameTable;
use crate::paths::{OutputArgs, write_file};

#[derive(Subcommand)]
pub enum CharCmd {
    /// Summarize a character file
    Info(InfoArgs),
    /// Change the character's name
    Rename(RenameArgs),
}

#[derive(Args)]
pub struct InfoArgs {
    /// Path to the .fch file
    pub file: PathBuf,
    /// Print as JSON
    #[arg(long)]
    pub json: bool,
    /// List every stored stat, not just the headline ones
    #[arg(long)]
    pub all_stats: bool,
}

#[derive(Args)]
pub struct RenameArgs {
    /// Path to the .fch file
    pub file: PathBuf,
    /// The new character name
    pub new_name: String,
    #[command(flatten)]
    pub out: OutputArgs,
}

pub fn run(cmd: CharCmd) -> Result<()> {
    match cmd {
        CharCmd::Info(args) => info(args),
        CharCmd::Rename(args) => rename(args),
    }
}

fn load(path: &PathBuf) -> Result<(CharacterFile, u64)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let file =
        CharacterFile::parse(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    Ok((file, bytes.len() as u64))
}

#[derive(Serialize)]
struct CharInfo {
    file: String,
    file_size: u64,
    hash_ok: bool,
    version: i32,
    name: String,
    player_id: i64,
    start_seed: String,
    date_created: Option<String>,
    used_cheats: Option<bool>,
    first_spawn: Option<bool>,
    stats: Vec<(String, f32)>,
    known_worlds: Vec<(String, f32)>,
    worlds: Vec<WorldEntry>,
    vitals: Option<Vitals>,
    foods: Vec<FoodEntry>,
    inventory: Vec<InventoryEntry>,
}

#[derive(Serialize)]
struct WorldEntry {
    world_uid: i64,
    spawn_point: Option<[f32; 3]>,
    logout_point: Option<[f32; 3]>,
    death_point: Option<[f32; 3]>,
    home_point: [f32; 3],
    map_data_bytes: usize,
}

#[derive(Serialize)]
struct Vitals {
    data_version: i32,
    max_health: Option<f32>,
    health: f32,
    max_stamina: Option<f32>,
    guardian_power: Option<String>,
}

#[derive(Serialize)]
struct FoodEntry {
    name: String,
    /// Seconds of digestion left, in saves new enough to store it.
    time_left: Option<f32>,
    health: Option<f32>,
    stamina: Option<f32>,
}

#[derive(Serialize)]
struct InventoryEntry {
    name: String,
    stack: i32,
    quality: i32,
    equipped: bool,
    crafter: String,
}

const HEADLINE_STATS: &[&str] = &[
    "Deaths",
    "EnemyKills",
    "BossKills",
    "Crafts",
    "Builds",
    "CreatureTamed",
    "DistanceTraveled",
];

fn info(args: InfoArgs) -> Result<()> {
    let (file, size) = load(&args.file)?;
    let p = &file.profile;

    let stats: Vec<(String, f32)> = p
        .stat_values()
        .iter()
        .enumerate()
        .map(|(i, v)| (stat_name(i), *v))
        .filter(|(name, v)| {
            args.all_stats || (HEADLINE_STATS.contains(&name.as_str()) && *v != 0.0)
        })
        .collect();

    let head = match file.player_data_head() {
        Some(Ok(head)) => Some(head),
        Some(Err(err)) => {
            eprintln!("warning: {err:#}; health, food and inventory are not shown");
            None
        }
        None => None,
    };
    let vitals = head.as_ref().map(|h| Vitals {
        data_version: h.version,
        max_health: h.max_health,
        health: h.health,
        max_stamina: h.max_stamina,
        guardian_power: h.guardian_power.as_ref().map(|s| s.0.clone()),
    });
    let foods: Vec<FoodEntry> = head
        .as_ref()
        .and_then(|h| h.foods.as_ref())
        .map(|f| {
            f.items
                .iter()
                .map(|food| FoodEntry {
                    name: food.name.0.clone(),
                    time_left: food.time,
                    health: food.health,
                    stamina: food.stamina,
                })
                .collect()
        })
        .unwrap_or_default();
    // Since 1.0 the player's inventory names items by prefab hash.
    let names = NameTable::builtin();
    let inventory: Vec<InventoryEntry> = head
        .as_ref()
        .map(|h| {
            h.inventory
                .items
                .iter()
                .map(|it| InventoryEntry {
                    name: it.name(&names),
                    stack: it.stack(),
                    quality: it.quality(),
                    equipped: it.equipped(),
                    crafter: it.crafter_name().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();

    let point = |have: bool, v: &crate::format::primitives::Vec3| {
        if have { Some([v.x, v.y, v.z]) } else { None }
    };
    let worlds = p
        .worlds
        .worlds
        .iter()
        .map(|w| WorldEntry {
            world_uid: w.world_uid,
            spawn_point: point(w.have_custom_spawn_point.0, &w.spawn_point),
            logout_point: point(w.have_logout_point.0, &w.logout_point),
            death_point: w
                .death
                .as_ref()
                .and_then(|d| point(d.have_death_point.0, &d.death_point)),
            home_point: [w.home_point.x, w.home_point.y, w.home_point.z],
            map_data_bytes: w
                .map_data
                .as_ref()
                .and_then(|m| m.data.as_ref())
                .map_or(0, |d| d.data.len()),
        })
        .collect();

    let info = CharInfo {
        file: args.file.display().to_string(),
        file_size: size,
        hash_ok: file.hash_ok,
        version: p.version,
        name: p.name.0.clone(),
        player_id: p.player_id,
        start_seed: p.start_seed.0.clone(),
        date_created: p.extended.as_ref().map(|e| unix_to_date(e.date_created)),
        used_cheats: p.extended.as_ref().map(|e| e.used_cheats.0),
        first_spawn: p.first_spawn.map(|b| b.0),
        stats,
        known_worlds: p
            .known_worlds()
            .iter()
            .map(|k| (k.key.0.clone(), k.value))
            .collect(),
        worlds,
        vitals,
        foods,
        inventory,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    println!("Character: {}", info.name);
    println!(
        "  file:          {} ({})",
        info.file,
        human_bytes(info.file_size)
    );
    println!(
        "  version:       {}{}",
        info.version,
        if info.hash_ok {
            ""
        } else {
            "  (stored hash does NOT match payload)"
        }
    );
    println!("  player id:     {}", info.player_id);
    println!("  start seed:    {}", info.start_seed);
    if let Some(d) = &info.date_created {
        println!("  created:       {d}");
    }
    if let Some(c) = info.used_cheats {
        println!("  used cheats:   {}", yes_no(c));
    }
    if let Some(v) = &info.vitals {
        let mh = v.max_health.map_or("?".into(), |x| format!("{x:.0}"));
        let ms = v.max_stamina.map_or("?".into(), |x| format!("{x:.0}"));
        println!(
            "  health:        {:.0} / {mh}   stamina: {ms}   (player data v{})",
            v.health, v.data_version
        );
        if let Some(gp) = &v.guardian_power
            && !gp.is_empty()
        {
            println!("  power:         {gp}");
        }
    }
    if !info.foods.is_empty() {
        println!("Food ({} slots in use):", info.foods.len());
        for f in &info.foods {
            println!("  {:<22} {}", f.name, food_detail(f));
        }
    }
    if !info.stats.is_empty() {
        println!("Stats:");
        for (name, v) in &info.stats {
            println!("  {name:<22} {}", format_stat(*v));
        }
    }
    if !info.known_worlds.is_empty() {
        println!("Known worlds (time played):");
        for (name, secs) in &info.known_worlds {
            println!("  {name:<22} {}", format_duration(*secs));
        }
    }
    if !info.worlds.is_empty() {
        println!("World positions:");
        for w in &info.worlds {
            let fmt = |p: &Option<[f32; 3]>| {
                p.map_or("-".to_owned(), |p| {
                    format!("({:.0}, {:.0}, {:.0})", p[0], p[1], p[2])
                })
            };
            println!(
                "  uid {:<20} logout {:<20} spawn {:<20} death {:<20} map {}",
                w.world_uid,
                fmt(&w.logout_point),
                fmt(&w.spawn_point),
                fmt(&w.death_point),
                human_bytes(w.map_data_bytes as u64)
            );
        }
    }
    if !info.inventory.is_empty() {
        println!("Inventory ({} items):", info.inventory.len());
        for it in &info.inventory {
            let q = if it.quality > 1 {
                format!(" q{}", it.quality)
            } else {
                String::new()
            };
            let eq = if it.equipped { " [equipped]" } else { "" };
            let by = if it.crafter.is_empty() {
                String::new()
            } else {
                format!("  by {}", it.crafter)
            };
            println!("  {:<28} x{}{q}{eq}{by}", it.name, it.stack);
        }
    }
    Ok(())
}

fn rename(args: RenameArgs) -> Result<()> {
    args.out.validate()?;
    let new_name = args.new_name.trim();
    if new_name.is_empty() {
        bail!("the new name must not be empty");
    }
    let (mut file, _) = load(&args.file)?;
    if !file.hash_ok {
        eprintln!(
            "warning: stored hash did not match the payload; the file may have been edited or damaged"
        );
    }
    let old_name = file.profile.name.0.clone();
    let old_version = file.profile.version;
    file.profile.name = new_name.into();
    file.profile.upgrade();
    let bytes = file.to_file_bytes()?;

    let target = args.out.output.clone().unwrap_or_else(|| args.file.clone());
    let backup = write_file(&target, &bytes, !args.out.no_backup)?;
    println!(
        "Renamed character \"{old_name}\" to \"{new_name}\" in {}",
        target.display()
    );
    if old_version != file.profile.version {
        println!(
            "  upgraded save format from version {old_version} to {}",
            file.profile.version
        );
    }
    if let Some(b) = backup {
        println!("  backup: {}", b.display());
    }
    println!("  note: the file name is unchanged; the game reads the name from inside the file");
    Ok(())
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// What is left of a serving: the remaining digestion time, or, in saves too
/// old to store it, the nutrition the item was still granting.
fn food_detail(f: &FoodEntry) -> String {
    if let Some(t) = f.time_left {
        return format!("{} left", format_duration(t));
    }
    match (f.health, f.stamina) {
        (Some(h), Some(s)) => format!("health {h:.0}, stamina {s:.0}"),
        (Some(h), None) => format!("health {h:.0}"),
        _ => "-".to_owned(),
    }
}

fn format_stat(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

fn format_duration(secs: f32) -> String {
    let total = secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{total}s")
    }
}
