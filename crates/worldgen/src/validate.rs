//! 世界不變量檢查。每次修改生成器後對大量種子批量運行。

use crate::hydro::Water;
use crate::places::{station_distance, MAX_STATION_DISTANCE};
use crate::settle::{min_distance, spacing, Style};
use crate::World;

#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub land_km2: f32,
    pub urban_fraction: f32,
    pub new_city_share: f32,
    pub settlements: usize,
    pub road_km: f32,
    pub street_km: f32,
    pub buildings: usize,
    pub stations: usize,
    pub pois: usize,
    pub max_station_km: f32,
    pub tallest_m: f32,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn check(w: &World) -> Report {
    let mut r = Report::default();
    let pg = &w.path_grid;
    let n = pg.n;
    r.land_km2 = w.land_area_m2 / 1e6;
    r.settlements = w.settlements.len();
    r.buildings = w.buildings.len();
    r.stations = w.stations.len();
    r.pois = w.pois.len();
    r.tallest_m = w.buildings.iter().map(|b| b.sy).fold(0.0, f32::max);

    if w.failed_links > 0 {
        r.errors.push(format!("{} 條道路尋路失敗", w.failed_links));
    }

    // 連通性
    let start = w.settlements.first().map(|s| pg.cell_of(s.x, s.z));
    if let Some((sx, sz)) = start {
        let mut seen = vec![false; n * n];
        let mut stack = vec![sz * n + sx];
        seen[sz * n + sx] = true;
        while let Some(k) = stack.pop() {
            let (x, y) = ((k % n) as i32, (k / n) as i32);
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                        continue;
                    }
                    let j = ny as usize * n + nx as usize;
                    if !seen[j] && pg.road[j] != 0 {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        for s in &w.settlements {
            let (x, z) = pg.cell_of(s.x, s.z);
            if !seen[z * n + x] {
                r.errors.push(format!("聚落 {} 未與路網連通", s.id));
            }
        }
    }

    // 城市面積
    let mut old = 0.0;
    let mut new = 0.0;
    for s in &w.settlements {
        match s.style {
            Style::Old => old += s.area_m2,
            Style::New => new += s.area_m2,
        }
    }
    r.urban_fraction = (old + new) / w.land_area_m2;
    r.new_city_share = new / (old + new).max(1.0);
    if !(0.24..=0.36).contains(&r.urban_fraction) {
        r.errors.push(format!("城市佔陸地比例 {:.2} 不在 0.24–0.36", r.urban_fraction));
    }
    if !(0.40..=0.60).contains(&r.new_city_share) {
        r.errors.push(format!("新城面積佔比 {:.2} 不在 0.40–0.60", r.new_city_share));
    }

    // 聚落間距
    let sp = spacing(w.size);
    for a in &w.settlements {
        let mut nearest = f32::MAX;
        for b in &w.settlements {
            if a.id == b.id {
                continue;
            }
            let d = libm::hypotf(a.x - b.x, a.z - b.z);
            nearest = nearest.min(d);
            if a.id < b.id && d < min_distance(a, b, &sp) - 1.0 {
                r.errors.push(format!("聚落 {} 與 {} 太近：{:.0} m", a.id, b.id, d));
            }
        }
        if w.settlements.len() > 1 && nearest > sp.max_nearest + 1.0 {
            r.errors.push(format!("聚落 {} 太孤立：最近鄰 {:.0} m", a.id, nearest));
        }
    }
    let cbds = w.settlements.iter().filter(|s| s.cbd.is_some()).count();
    let new_majors = w.settlements.iter().filter(|s| s.style == Style::New && s.tier == crate::settle::Tier::Major).count();
    if cbds != new_majors {
        r.errors.push(format!("CBD 數量 {} 與新城大城市數量 {} 不符", cbds, new_majors));
    }

    // 道路坡度
    for (i, road) in w.roads.iter().enumerate() {
        let g = road.class.max_grade();
        for p in road.pts.windows(2) {
            let d = libm::hypotf(p[1][0] - p[0][0], p[1][2] - p[0][2]).max(0.01);
            let gr = libm::fabsf(p[1][1] - p[0][1]) / d;
            if gr > g + 0.01 {
                r.errors.push(format!("道路 {} 坡度 {:.3} 超過 {:.2}", i, gr, g));
                break;
            }
        }
        r.road_km += road.pts.windows(2).map(|p| libm::hypotf(p[1][0] - p[0][0], p[1][2] - p[0][2])).sum::<f32>() / 1000.0;
    }
    for s in &w.streets {
        r.street_km += s.pts.windows(2).map(|p| libm::hypotf(p[1][0] - p[0][0], p[1][2] - p[0][2])).sum::<f32>() / 1000.0;
    }

    // 加油站覆蓋
    let dist = station_distance(pg, &w.stations);
    let mut worst = 0.0f32;
    for k in 0..n * n {
        if pg.road[k] != 0 {
            worst = worst.max(dist[k]);
        }
    }
    r.max_station_km = worst / 1000.0;
    if worst > MAX_STATION_DISTANCE {
        r.errors.push(format!("離最近加油站最遠 {:.1} km，超過 15 km", worst / 1000.0));
    }

    // 建築不在水中
    let wet = w
        .buildings
        .iter()
        .filter(|b| {
            let (cx, cz) = w.hydro.kind.cell_of(b.x, b.z);
            w.hydro.kind.get_clamped(cx, cz) != Water::None as u8
        })
        .count();
    if wet > 0 {
        r.errors.push(format!("{} 棟建築位於水中", wet));
    }
    r
}
