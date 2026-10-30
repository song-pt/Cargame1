//! 加油站與沿路景點。

use crate::city::{Building, BuildingKind};
use crate::grid::Grid;
use crate::hydro::{Hydro, Water};
use crate::roads::{PathGrid, Road, RoadClass};
use crate::rng::Rng;
use crate::settle::Settlement;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// 主要道路上任何位置到最近加油站的最大行駛距離
pub const MAX_STATION_DISTANCE: f32 = 15_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoiKind {
    Viewpoint,
    Lighthouse,
    Shrine,
}

#[derive(Clone, Copy, Debug)]
pub struct Poi {
    pub kind: PoiKind,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Station {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub angle: f32,
}

fn is_main(c: RoadClass) -> bool {
    matches!(c, RoadClass::Highway | RoadClass::Regional | RoadClass::Mountain)
}

/// 在道路旁放一座站：優先右側，右側是水就放左側
fn station_at(road: &Road, i: usize, h: &Grid<f32>, hy: &Hydro) -> Station {
    let i = i.clamp(1, road.pts.len() - 2);
    let a = road.pts[i - 1];
    let b = road.pts[i + 1];
    let (tx, tz) = (b[0] - a[0], b[2] - a[2]);
    let l = libm::hypotf(tx, tz).max(1e-3);
    let (nx, nz) = (-tz / l, tx / l);
    let off = road.width * 0.5 + 14.0;
    let p = road.pts[i];
    let wet = |x: f32, z: f32| {
        let (cx, cz) = hy.kind.cell_of(x, z);
        hy.kind.get_clamped(cx, cz) != Water::None as u8
    };
    let side = if wet(p[0] + nx * off, p[2] + nz * off) { -1.0 } else { 1.0 };
    let (x, z) = (p[0] + nx * off * side, p[2] + nz * off * side);
    Station { x, y: h.sample(x, z), z, angle: libm::atan2f(nz * side, nx * side) }
}

#[derive(PartialEq)]
struct D(f32, u32);
impl Eq for D {}
impl PartialOrd for D {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for D {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

/// 道路格上，到最近加油站的行駛距離
pub fn station_distance(pg: &PathGrid, stations: &[Station]) -> Vec<f32> {
    let n = pg.n;
    let mut dist = vec![f32::INFINITY; n * n];
    let mut heap = BinaryHeap::new();
    for s in stations {
        let (cx, cz) = pg.cell_of(s.x, s.z);
        // 站在路旁，吸附到 2 格內最近的道路格
        let mut best = None;
        let mut bd = f32::MAX;
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let (x, y) = (cx as i32 + dx, cz as i32 + dy);
                if x < 0 || y < 0 || x >= n as i32 || y >= n as i32 {
                    continue;
                }
                let k = y as usize * n + x as usize;
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if pg.road[k] != 0 && d < bd {
                    bd = d;
                    best = Some(k);
                }
            }
        }
        if let Some(k) = best {
            dist[k] = 0.0;
            heap.push(D(0.0, k as u32));
        }
    }
    while let Some(D(d, k)) = heap.pop() {
        let k = k as usize;
        if d > dist[k] {
            continue;
        }
        let (x, y) = ((k % n) as i32, (k / n) as i32);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                continue;
            }
            let j = ny as usize * n + nx as usize;
            if pg.road[j] == 0 {
                continue;
            }
            let nd = d + if dx != 0 && dy != 0 { 1.4142 } else { 1.0 } * pg.cell;
            if nd < dist[j] {
                dist[j] = nd;
                heap.push(D(nd, j as u32));
            }
        }
    }
    dist
}

pub fn place_stations(roads: &[Road], pg: &PathGrid, settlements: &[Settlement], h: &Grid<f32>, hy: &Hydro) -> Vec<Station> {
    let mut stations = Vec::new();
    for s in settlements {
        let mut best: Option<(f32, usize, usize)> = None;
        for (ri, r) in roads.iter().enumerate() {
            if !is_main(r.class) || r.pts.len() < 3 {
                continue;
            }
            for (i, p) in r.pts.iter().enumerate() {
                if r.bridge[i] {
                    continue;
                }
                let d = libm::hypotf(p[0] - s.x, p[2] - s.z);
                // 不要正好在市中心：偏好半徑 40% 附近
                let score = libm::fabsf(d - s.radius * 0.4);
                if best.map_or(true, |b| score < b.0) {
                    best = Some((score, ri, i));
                }
            }
        }
        if let Some((_, ri, i)) = best {
            stations.push(station_at(&roads[ri], i, h, hy));
        }
    }
    // 覆蓋檢查：不足處補站
    for _ in 0..32 {
        let dist = station_distance(pg, &stations);
        let mut worst = (0.0f32, 0usize);
        for (k, &d) in dist.iter().enumerate() {
            if pg.road[k] != 0 && d.is_finite() && d > worst.0 {
                worst = (d, k);
            }
        }
        if worst.0 <= MAX_STATION_DISTANCE * 0.85 {
            break;
        }
        let (wx, wz) = (((worst.1 % pg.n) as f32 + 0.5) * pg.cell, ((worst.1 / pg.n) as f32 + 0.5) * pg.cell);
        let mut best: Option<(f32, usize, usize)> = None;
        for (ri, r) in roads.iter().enumerate() {
            if r.pts.len() < 3 {
                continue;
            }
            for (i, p) in r.pts.iter().enumerate() {
                let d = libm::hypotf(p[0] - wx, p[2] - wz);
                if !r.bridge[i] && best.map_or(true, |b| d < b.0) {
                    best = Some((d, ri, i));
                }
            }
        }
        match best {
            Some((_, ri, i)) => stations.push(station_at(&roads[ri], i, h, hy)),
            None => break,
        }
    }
    stations
}

pub fn place_pois(roads: &[Road], h: &Grid<f32>, hy: &Hydro, urban: &Grid<u8>, pg: &PathGrid, seed: u64) -> Vec<Poi> {
    let mut rng = Rng::derive(seed, 0x9017);
    let mut pois: Vec<Poi> = Vec::new();
    let near_poi = |pois: &[Poi], x: f32, z: f32, r: f32| pois.iter().any(|p| libm::hypotf(p.x - x, p.z - z) < r);
    for road in roads.iter().filter(|r| is_main(r.class)) {
        // 山道終點一定有觀景台
        if road.class == RoadClass::Mountain {
            let p = *road.pts.last().unwrap();
            let (cx, cz) = hy.kind.cell_of(p[0], p[2]);
            if !near_poi(&pois, p[0], p[2], 300.0) && hy.kind.get_clamped(cx, cz) == Water::None as u8 {
                pois.push(Poi { kind: PoiKind::Viewpoint, x: p[0], y: p[1], z: p[2] });
            }
        }
        let mut acc = 0.0;
        let mut next = rng.range(1200.0, 1800.0);
        for i in 1..road.pts.len() {
            let a = road.pts[i - 1];
            let b = road.pts[i];
            acc += libm::hypotf(b[0] - a[0], b[2] - a[2]);
            if acc < next {
                continue;
            }
            acc = 0.0;
            next = rng.range(1200.0, 1800.0);
            let (cx, cz) = urban.cell_of(b[0], b[2]);
            if urban.get_clamped(cx, cz) != 0 || near_poi(&pois, b[0], b[2], 700.0) {
                continue;
            }
            // 在道路兩側找候選點
            let (tx, tz) = (b[0] - a[0], b[2] - a[2]);
            let l = libm::hypotf(tx, tz).max(1e-3);
            let (nx, nz) = (-tz / l, tx / l);
            let mut best: Option<(f32, PoiKind, f32, f32)> = None;
            for side in [-1.0f32, 1.0] {
                for off in [40.0f32, 90.0, 160.0, 240.0] {
                    let (px, pz) = (b[0] + nx * off * side, b[2] + nz * off * side);
                    let (gx, gz) = h.cell_of(px, pz);
                    if !h.in_bounds(gx, gz) || hy.kind.get(gx as usize, gz as usize) != Water::None as u8 {
                        continue;
                    }
                    let (qx, qz) = pg.cell_of(px, pz);
                    if pg.road[qz * pg.n + qx] != 0 {
                        continue;
                    }
                    let elev = h.sample(px, pz);
                    let slope = h.slope_at(gx as usize, gz as usize);
                    if slope > 0.35 {
                        continue;
                    }
                    let coast = pg.coast[qz * pg.n + qx];
                    let (kind, score) = if coast && elev > 6.0 {
                        (PoiKind::Lighthouse, 2.0 + elev / 40.0)
                    } else if elev > b[1] + 8.0 || elev > 220.0 {
                        (PoiKind::Viewpoint, 1.0 + (elev - b[1]) / 30.0)
                    } else {
                        (PoiKind::Shrine, 0.8 + rng.f32() * 0.3)
                    };
                    let score = score - off / 400.0;
                    if best.map_or(true, |bb| score > bb.0) {
                        best = Some((score, kind, px, pz));
                    }
                }
            }
            if let Some((_, kind, px, pz)) = best {
                if kind == PoiKind::Lighthouse && pois.iter().filter(|p| p.kind == PoiKind::Lighthouse).any(|p| libm::hypotf(p.x - px, p.z - pz) < 2500.0) {
                    pois.push(Poi { kind: PoiKind::Viewpoint, x: px, y: h.sample(px, pz), z: pz });
                } else {
                    pois.push(Poi { kind, x: px, y: h.sample(px, pz), z: pz });
                }
            }
        }
    }
    pois
}

pub fn poi_buildings(pois: &[Poi], stations: &[Station], seed: u64) -> Vec<Building> {
    let mut rng = Rng::derive(seed, 0xB0B);
    let mut out = Vec::new();
    for s in stations {
        out.push(Building { x: s.x, y: s.y - 0.3, z: s.z, angle: s.angle, sx: 22.0, sy: 6.5, sz: 16.0, kind: BuildingKind::GasStation, seed: rng.next_u64() as u32 });
    }
    for p in pois {
        let (sx, sy, sz, kind) = match p.kind {
            PoiKind::Viewpoint => (8.0, 4.5, 8.0, BuildingKind::Pavilion),
            PoiKind::Lighthouse => (5.0, 22.0, 5.0, BuildingKind::Lighthouse),
            PoiKind::Shrine => (6.0, 5.5, 1.2, BuildingKind::Shrine),
        };
        out.push(Building { x: p.x, y: p.y - 0.3, z: p.z, angle: rng.range(0.0, 6.283), sx, sy, sz, kind, seed: rng.next_u64() as u32 });
    }
    out
}
