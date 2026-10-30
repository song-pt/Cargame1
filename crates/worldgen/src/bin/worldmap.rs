//! 命令列工具：生成世界、驗證、輸出俯視地圖 PNG。
//!
//! 用法：worldmap [--seed N] [--size small|medium|large] [--out map.png] [--px 1024]
//!       worldmap --batch 0..20 [--size medium]   批量驗證

use std::time::Instant;
use worldgen::{generate, map, validate, WorldSize, OFFICIAL_SEED};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let size = WorldSize::from_str(&get("--size").unwrap_or_else(|| "medium".into()));
    let t0 = Instant::now();
    let now = move || t0.elapsed().as_secs_f64() * 1000.0;

    if let Some(range) = get("--batch") {
        let (a, b) = range.split_once("..").expect("--batch a..b");
        let (a, b): (u64, u64) = (a.parse().unwrap(), b.parse().unwrap());
        let mut failed = 0;
        for seed in a..b {
            let s0 = now();
            let w = generate(seed, size, &now);
            let r = validate::check(&w);
            println!(
                "seed {:>4}: {:>6.0} ms  land {:>5.1} km²  urban {:.2}  new {:.2}  roads {:>5.1} km  bldg {:>6}  stations {:>2}  pois {:>2}  {}",
                seed,
                now() - s0,
                r.land_km2,
                r.urban_fraction,
                r.new_city_share,
                r.road_km,
                r.buildings,
                r.stations,
                r.pois,
                if r.ok() { "OK".to_string() } else { format!("FAIL: {}", r.errors.join("; ")) }
            );
            if !r.ok() {
                failed += 1;
            }
        }
        println!("{} / {} 通過", (b - a) as usize - failed, b - a);
        std::process::exit(if failed > 0 { 1 } else { 0 });
    }

    let seed: u64 = get("--seed").map(|s| s.parse().unwrap()).unwrap_or(OFFICIAL_SEED);
    let out = get("--out").unwrap_or_else(|| "map.png".into());
    let px: usize = get("--px").map(|s| s.parse().unwrap()).unwrap_or(1024);
    let w = generate(seed, size, &now);
    for (name, ms) in &w.timings {
        println!("  {:<12} {:>7.0} ms", name, ms);
    }
    let r = validate::check(&w);
    println!("{:#?}", r);
    let img = map::render(&w, px);
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.w as u32, img.h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&img.rgba).unwrap();
    println!("寫入 {}", out);
}
