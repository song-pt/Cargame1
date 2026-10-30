//! 聚落：選址、間距約束、城市範圍、CBD。

use crate::grid::Grid;
use crate::hydro::{Hydro, Water};
use crate::noise::{fbm, lerp};
use crate::rng::Rng;
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// 老城：台北 / 香港 / 90 年代東京
    Old,
    /// 新城：中國大陸新城市，帶唯一 CBD
    New,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    Major,
    Mid,
    Town,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TownKind {
    City,
    Fishing,
    Onsen,
}

#[derive(Clone, Debug)]
pub struct Cbd {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
}

#[derive(Clone, Debug)]
pub struct Settlement {
    pub id: usize,
    pub x: f32,
    pub z: f32,
    /// 選址時使用的規劃半徑（間距約束以此計算）
    pub plan_radius: f32,
    /// 調整後的實際範圍半徑
    pub radius: f32,
    pub style: Style,
    pub tier: Tier,
    pub kind: TownKind,
    pub street_angle: f32,
    pub cbd: Option<Cbd>,
    pub area_m2: f32,
}

pub struct Spacing {
    pub min_factor: f32,
    pub max_nearest: f32,
}

pub fn spacing(size: f32) -> Spacing {
    Spacing { min_factor: 1.15, max_nearest: 5500.0 * size / 12288.0 }
}

pub fn min_distance(a: &Settlement, b: &Settlement, sp: &Spacing) -> f32 {
    sp.min_factor * (a.plan_radius + b.plan_radius)
}

/// 到某類格子的距離（格數，chamfer 近似）
pub fn distance_field(n: usize, src: impl Fn(usize) -> bool) -> Vec<u16> {
    let mut d = vec![u16::MAX; n * n];
    let mut q = VecDeque::new();
    for k in 0..n * n {
        if src(k) {
            d[k] = 0;
            q.push_back(k);
        }
    }
    while let Some(k) = q.pop_front() {
        let (x, y) = ((k % n) as i32, (k / n) as i32);
        let nd = d[k] + 1;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && (nx as usize) < n && (ny as usize) < n {
                let j = ny as usize * n + nx as usize;
                if d[j] > nd {
                    d[j] = nd;
                    q.push_back(j);
                }
            }
        }
    }
    d
}

struct Spec {
    tier: Tier,
    style: Style,
    kind: TownKind,
    area: f32,
}

pub struct SettleOut {
    pub list: Vec<Settlement>,
    /// 0 = 非城市，否則為 settlement id + 1
    pub urban: Grid<u8>,
    pub land_area_m2: f32,
}

pub fn place(h: &mut Grid<f32>, hy: &Hydro, size: f32, seed: u64) -> SettleOut {
    let n = h.n;
    let cell = h.cell;
    let sf = size / 12288.0;
    let mut rng = Rng::derive(seed, 0x5E77);

    let land = |k: usize| hy.kind.data[k] == Water::None as u8 && h.data[k] > 0.5;
    let land_cells = (0..n * n).filter(|&k| land(k)).count();
    let land_area = land_cells as f32 * cell * cell;

    let slope = {
        let mut s = Grid::new(n, cell, 0.0f32);
        for y in 0..n {
            for x in 0..n {
                s.data[y * n + x] = h.slope_at(x, y);
            }
        }
        s.blurred(3)
    };
    let d_fresh = distance_field(n, |k| {
        let w = hy.kind.data[k];
        w == Water::River as u8 || w == Water::Lake as u8
    });
    let d_ocean = distance_field(n, |k| hy.kind.data[k] == Water::Ocean as u8);

    // 城市面積目標：陸地 30%，新老各半
    let half_share = 0.15 * land_area;
    let mids = ((2.0 * sf * sf).round() as usize).clamp(1, 4);
    let towns = ((3.0 * sf * sf).round() as usize).clamp(1, 6);
    let mut specs = vec![
        Spec { tier: Tier::Major, style: Style::New, kind: TownKind::City, area: half_share * 0.72 },
        Spec { tier: Tier::Major, style: Style::Old, kind: TownKind::City, area: half_share * 0.62 },
    ];
    let new_mids = mids / 2;
    let old_mids = mids - new_mids;
    for i in 0..mids {
        let style = if i < old_mids { Style::Old } else { Style::New };
        let share = if style == Style::Old { 0.22 / old_mids as f32 } else { 0.28 / new_mids.max(1) as f32 };
        specs.push(Spec { tier: Tier::Mid, style, kind: TownKind::City, area: half_share * share });
    }
    for i in 0..towns {
        let kind = match i % 3 {
            0 => TownKind::Fishing,
            1 => TownKind::Onsen,
            _ => TownKind::City,
        };
        specs.push(Spec { tier: Tier::Town, style: Style::Old, kind, area: half_share * 0.16 / towns as f32 });
    }

    let sp = spacing(size);
    let mut list: Vec<Settlement> = Vec::new();
    for (si, spec) in specs.iter().enumerate() {
        let plan_r = libm::sqrtf(spec.area / (core::f32::consts::PI * 0.72));
        let mut best: Option<(f32, f32, f32)> = None;
        for attempt in 0..3 {
            let relax = 1.0 + attempt as f32 * 0.6;
            for _ in 0..900 {
                let x = rng.below(n);
                let y = rng.below(n);
                let k = y * n + x;
                if !land(k) {
                    continue;
                }
                let (wx, wz) = h.center(x, y);
                let elev = h.data[k];
                let (max_slope, max_elev) = match spec.kind {
                    TownKind::Onsen => (0.16, 480.0),
                    _ => (0.07, 160.0),
                };
                let s = slope.data[k];
                if s > max_slope * relax || elev > max_elev * relax {
                    continue;
                }
                let probe = Settlement {
                    id: 0,
                    x: wx,
                    z: wz,
                    plan_radius: plan_r,
                    radius: plan_r,
                    style: spec.style,
                    tier: spec.tier,
                    kind: spec.kind,
                    street_angle: 0.0,
                    cbd: None,
                    area_m2: 0.0,
                };
                let mut ok = true;
                let mut nearest = f32::MAX;
                for o in &list {
                    let d = libm::hypotf(o.x - wx, o.z - wz);
                    if d < min_distance(o, &probe, &sp) {
                        ok = false;
                        break;
                    }
                    nearest = nearest.min(d);
                }
                if !ok || (!list.is_empty() && nearest > sp.max_nearest) {
                    continue;
                }
                // 範圍內的平坦度
                let mut flat = 0.0;
                let mut cnt = 0.0;
                let mut wet = 0.0;
                for &(ox, oz) in &[(0.0, 0.0), (0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5), (0.35, 0.35), (-0.35, -0.35), (0.35, -0.35), (-0.35, 0.35)] {
                    let (cx, cz) = h.cell_of(wx + ox * plan_r, wz + oz * plan_r);
                    if h.in_bounds(cx, cz) {
                        let j = cz as usize * n + cx as usize;
                        flat += (1.0 - slope.data[j] / 0.12).max(-1.0);
                        if !land(j) {
                            wet += 1.0;
                        }
                        cnt += 1.0;
                    }
                }
                let mut score = flat / cnt - wet * 0.12;
                let fresh_m = d_fresh[k] as f32 * cell;
                let ocean_m = d_ocean[k] as f32 * cell;
                score += match (spec.style, spec.kind) {
                    (Style::New, _) => 1.2 * (1.0 - fresh_m / 1500.0).max(0.0),
                    (_, TownKind::Fishing) => 1.5 * (1.0 - ocean_m / 500.0).max(0.0),
                    (_, TownKind::Onsen) => (elev / 300.0).min(1.0) + 0.5 * (1.0 - fresh_m / 1500.0).max(0.0),
                    _ => 0.6 * (1.0 - ocean_m / 2000.0).max(0.0) + 0.4 * (1.0 - fresh_m / 1500.0).max(0.0),
                };
                score += rng.f32() * 0.25;
                if best.map_or(true, |b| score > b.0) {
                    best = Some((score, wx, wz));
                }
            }
            if best.is_some() {
                break;
            }
        }
        if let Some((_, wx, wz)) = best {
            list.push(Settlement {
                id: list.len(),
                x: wx,
                z: wz,
                plan_radius: plan_r,
                radius: plan_r,
                style: spec.style,
                tier: spec.tier,
                kind: spec.kind,
                street_angle: rng.range(0.0, core::f32::consts::FRAC_PI_2),
                cbd: None,
                area_m2: 0.0,
            });
        } else {
            let _ = si;
        }
    }

    // 範圍與面積調整：反覆計算實際面積，讓新老城面積各接近 15%
    let mut urban = Grid::new(n, cell, 0u8);
    let nseed = (seed as u32) ^ 0x0BA0;
    for _iter in 0..5 {
        assign_footprints(h, hy, &slope, &list, &mut urban, nseed);
        let mut area = vec![0.0f32; list.len()];
        for &u in &urban.data {
            if u > 0 {
                area[(u - 1) as usize] += cell * cell;
            }
        }
        let old: f32 = list.iter().zip(&area).filter(|(s, _)| s.style == Style::Old).map(|(_, a)| a).sum();
        let new: f32 = list.iter().zip(&area).filter(|(s, _)| s.style == Style::New).map(|(_, a)| a).sum();
        for (s, a) in list.iter_mut().zip(&area) {
            s.area_m2 = *a;
            let total = if s.style == Style::Old { old } else { new };
            let k = libm::sqrtf(half_share / total.max(1.0)).clamp(0.85, 1.2);
            s.radius = (s.radius * k).clamp(s.plan_radius * 0.75, s.plan_radius * 1.5);
        }
    }
    assign_footprints(h, hy, &slope, &list, &mut urban, nseed);
    let mut area = vec![0.0f32; list.len()];
    for &u in &urban.data {
        if u > 0 {
            area[(u - 1) as usize] += cell * cell;
        }
    }
    for (s, a) in list.iter_mut().zip(&area) {
        s.area_m2 = *a;
    }

    // CBD：新城大城市，靠河、靠中心
    for s in list.iter_mut() {
        if s.style != Style::New || s.tier != Tier::Major {
            continue;
        }
        let mut best = (f32::MIN, s.x, s.z);
        let r = s.radius * 0.55;
        let steps = 24;
        for iy in 0..=steps {
            for ix in 0..=steps {
                let wx = s.x - r + 2.0 * r * ix as f32 / steps as f32;
                let wz = s.z - r + 2.0 * r * iy as f32 / steps as f32;
                let (cx, cz) = h.cell_of(wx, wz);
                if !h.in_bounds(cx, cz) {
                    continue;
                }
                let k = cz as usize * n + cx as usize;
                if urban.data[k] as usize != s.id + 1 {
                    continue;
                }
                let fresh_m = d_fresh[k] as f32 * cell;
                let river = if fresh_m < 120.0 { -1.0 } else { (1.0 - (fresh_m - 150.0).abs() / 600.0).max(0.0) };
                let center = 1.0 - libm::hypotf(wx - s.x, wz - s.z) / r;
                let score = river * 1.2 + center;
                if score > best.0 {
                    best = (score, wx, wz);
                }
            }
        }
        s.cbd = Some(Cbd { x: best.1, z: best.2, radius: s.radius * 0.3 });
    }

    // 城市地面整平
    let blur = h.blurred(8);
    for k in 0..n * n {
        if urban.data[k] == 0 {
            continue;
        }
        let near_water = d_fresh[k] <= 4 || d_ocean[k] <= 3;
        let v = lerp(h.data[k], blur.data[k], 0.75);
        h.data[k] = if near_water { v.max(h.data[k]) } else { v };
    }

    SettleOut { list, urban, land_area_m2: land_area }
}

fn assign_footprints(h: &Grid<f32>, hy: &Hydro, slope: &Grid<f32>, list: &[Settlement], urban: &mut Grid<u8>, nseed: u32) {
    let n = h.n;
    let cell = h.cell;
    let mut best = vec![f32::MAX; n * n];
    for v in urban.data.iter_mut() {
        *v = 0;
    }
    for s in list {
        let ext = s.radius * 1.35;
        let (x0, y0) = h.cell_of(s.x - ext, s.z - ext);
        let (x1, y1) = h.cell_of(s.x + ext, s.z + ext);
        for y in y0.max(0)..=y1.min(n as i32 - 1) {
            for x in x0.max(0)..=x1.min(n as i32 - 1) {
                let k = y as usize * n + x as usize;
                if hy.kind.data[k] != Water::None as u8 || h.data[k] < 0.8 || slope.data[k] > 0.2 {
                    continue;
                }
                let (wx, wz) = h.center(x as usize, y as usize);
                let warp = 1.0 + 0.25 * fbm(wx / 650.0, wz / 650.0, nseed.wrapping_add(s.id as u32 * 31), 3);
                let d = libm::hypotf(wx - s.x, wz - s.z) / (s.radius * warp);
                if d < 1.0 && d < best[k] {
                    best[k] = d;
                    urban.data[k] = (s.id + 1) as u8;
                }
            }
        }
    }
    let _ = cell;
}
