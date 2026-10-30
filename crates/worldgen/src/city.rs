//! 城市內部：街道網格與建築。老城密、窄、亂；新城規整的小區 + 唯一 CBD。

use crate::grid::Grid;
use crate::hydro::{Hydro, Water};
use crate::noise::{fbm, lerp};
use crate::rng::Rng;
use crate::roads::{build_profile, PathGrid, Road, RoadClass};
use crate::settle::{Settlement, Style, Tier, TownKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum BuildingKind {
    OldShop = 0,
    NewResidential = 1,
    Tower = 2,
    Podium = 3,
    TownHouse = 4,
    GasStation = 5,
    Shrine = 6,
    Lighthouse = 7,
    Pavilion = 8,
    Landmark = 9,
}

#[derive(Clone, Copy, Debug)]
pub struct Building {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 繞 Y 軸旋轉；局部 +Z 朝向街道
    pub angle: f32,
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
    pub kind: BuildingKind,
    pub seed: u32,
}

struct Lines {
    /// (位置, 道路等級)
    a: Vec<(f32, RoadClass)>,
    b: Vec<(f32, RoadClass)>,
}

fn line_set(s: &Settlement, rng: &mut Rng) -> Lines {
    let r = s.radius * 1.3;
    let mk = |rng: &mut Rng| -> Vec<(f32, RoadClass)> {
        let mut v = Vec::new();
        match s.style {
            Style::Old => {
                let mut p = -r;
                while p < r {
                    v.push((p, RoadClass::Street));
                    let gap = rng.range(70.0, 105.0);
                    if s.tier != Tier::Town && rng.f32() < 0.45 {
                        v.push((p + gap * 0.5, RoadClass::Alley));
                    }
                    p += gap;
                }
            }
            Style::New => {
                let mut p = -(r / 300.0).floor() * 300.0;
                let mut i = 0;
                while p < r {
                    v.push((p, if i % 2 == 0 { RoadClass::Arterial } else { RoadClass::Street }));
                    p += 300.0;
                    i += 1;
                }
            }
        }
        v
    };
    let a = mk(rng);
    let b = mk(rng);
    Lines { a, b }
}

pub struct CityOut {
    pub streets: Vec<Road>,
    pub buildings: Vec<Building>,
}

pub fn generate(
    h: &Grid<f32>,
    hy: &Hydro,
    urban: &Grid<u8>,
    pg: &PathGrid,
    settlements: &[Settlement],
    seed: u64,
) -> CityOut {
    let mut streets = Vec::new();
    let mut buildings = Vec::new();
    for s in settlements {
        let mut rng = Rng::derive(seed, 0xC17 + s.id as u64);
        let lines = line_set(s, &mut rng);
        let (ca, sa) = (libm::cosf(s.street_angle), libm::sinf(s.street_angle));
        let to_world = |u: f32, v: f32| -> (f32, f32) { (s.x + u * ca - v * sa, s.z + u * sa + v * ca) };
        let inside = |wx: f32, wz: f32| -> bool {
            let (cx, cz) = urban.cell_of(wx, wz);
            urban.in_bounds(cx, cz) && urban.get(cx as usize, cz as usize) as usize == s.id + 1
        };
        let wet = |wx: f32, wz: f32| -> bool {
            let (cx, cz) = hy.kind.cell_of(wx, wz);
            hy.kind.get_clamped(cx, cz) != Water::None as u8
        };
        let wobble_amp = if s.style == Style::Old { 4.0 } else { 0.0 };
        let wseed = seed as u32 ^ (s.id as u32 * 977);
        let r = s.radius * 1.3;

        // 街道
        for (family, set) in [(0, &lines.a), (1, &lines.b)] {
            for &(c, class) in set.iter() {
                let mut flags = Vec::new();
                let mut pts = Vec::new();
                let mut t = -r;
                while t <= r {
                    let wob = wobble_amp * fbm(t / 260.0, c / 97.0, wseed + family, 2);
                    let (u, v) = if family == 0 { (t, c + wob) } else { (c + wob, t) };
                    let (wx, wz) = to_world(u, v);
                    pts.push([wx, wz]);
                    flags.push(inside(wx, wz));
                    t += 10.0;
                }
                if class == RoadClass::Arterial {
                    // 主幹道以橋跨過短的水面缺口
                    let m = flags.len();
                    let mut i = 0;
                    while i < m {
                        if !flags[i] {
                            let mut e = i;
                            while e < m && !flags[e] {
                                e += 1;
                            }
                            if i > 0 && e < m && e - i <= 25 && (i..e).all(|j| wet(pts[j][0], pts[j][1])) {
                                for f in flags.iter_mut().take(e).skip(i) {
                                    *f = true;
                                }
                            }
                            i = e;
                        } else {
                            i += 1;
                        }
                    }
                }
                let mut i = 0;
                while i < flags.len() {
                    if flags[i] {
                        let mut e = i;
                        while e < flags.len() && flags[e] {
                            e += 1;
                        }
                        if e - i >= 4 {
                            let run = &pts[i..e];
                            streets.push(build_profile(run, class, h, hy, None, None));
                        }
                        i = e;
                    } else {
                        i += 1;
                    }
                }
            }
        }

        // 建築：在線網格切出的街區裡放置
        let cbd = s.cbd.clone();
        let mut landmark_done = false;
        let mut all_a: Vec<(f32, RoadClass)> = lines.a.clone();
        let mut all_b: Vec<(f32, RoadClass)> = lines.b.clone();
        all_a.sort_by(|x, y| x.0.total_cmp(&y.0));
        all_b.sort_by(|x, y| x.0.total_cmp(&y.0));
        for wa in all_a.windows(2) {
            for wb in all_b.windows(2) {
                let inset = |c: RoadClass| c.width() * 0.5 + 2.5 + wobble_amp;
                let v0 = wa[0].0 + inset(wa[0].1);
                let v1 = wa[1].0 - inset(wa[1].1);
                let u0 = wb[0].0 + inset(wb[0].1);
                let u1 = wb[1].0 - inset(wb[1].1);
                if v1 - v0 < 10.0 || u1 - u0 < 10.0 {
                    continue;
                }
                let (bcx, bcz) = to_world((u0 + u1) * 0.5, (v0 + v1) * 0.5);
                if !inside(bcx, bcz) {
                    continue;
                }
                let dist_c = libm::hypotf(bcx - s.x, bcz - s.z) / s.radius;
                let in_cbd = cbd.as_ref().map_or(false, |c| libm::hypotf(bcx - c.x, bcz - c.z) < c.radius);
                let mut lots: Vec<(f32, f32, f32, f32, f32, BuildingKind)> = Vec::new(); // u, v, w, d, h, kind（u,v 為中心）
                match (s.style, in_cbd) {
                    (Style::New, true) => {
                        let c = cbd.as_ref().unwrap();
                        let dc = libm::hypotf(bcx - c.x, bcz - c.z) / c.radius;
                        // 150 m 子街區，每個 1–2 棟塔樓
                        let mut uu = u0;
                        while uu + 40.0 <= u1 {
                            let mut vv = v0;
                            let cw = ((u1 - uu) / 2.0).clamp(45.0, 140.0);
                            while vv + 40.0 <= v1 {
                                let cd = ((v1 - vv) / 2.0).clamp(45.0, 140.0);
                                let fw = (cw * rng.range(0.45, 0.7)).clamp(30.0, 62.0);
                                let fd = (cd * rng.range(0.45, 0.7)).clamp(30.0, 62.0);
                                let ht = lerp(470.0, 160.0, dc.clamp(0.0, 1.0)) * rng.range(0.6, 1.08);
                                lots.push((uu + cw * 0.5, vv + cd * 0.5, fw, fd, ht.max(120.0), BuildingKind::Tower));
                                vv += cd;
                            }
                            uu += cw;
                        }
                        if !landmark_done && dc < 0.45 {
                            landmark_done = true;
                            lots.clear();
                            let cu = (u0 + u1) * 0.5;
                            let cv = (v0 + v1) * 0.5;
                            lots.push((cu, cv, 66.0, 66.0, rng.range(520.0, 600.0), BuildingKind::Landmark));
                        }
                    }
                    (Style::New, false) => {
                        // 小區：臨主幹道一圈裙樓，內部排排住宅塔樓
                        let pod = 14.0;
                        lots.push(((u0 + u1) * 0.5, v0 + pod * 0.5, u1 - u0, pod, rng.range(6.0, 10.0), BuildingKind::Podium));
                        let mut vv = v0 + pod + 22.0;
                        while vv + 16.0 < v1 - 8.0 {
                            let mut uu = u0 + 14.0;
                            while uu + 24.0 < u1 - 8.0 {
                                let floors = (rng.range(18.0, 33.0)).round();
                                lots.push((uu + 12.0, vv + 8.0, 24.0, 16.0, floors * 3.0, BuildingKind::NewResidential));
                                uu += rng.range(36.0, 44.0);
                            }
                            vv += 56.0;
                        }
                    }
                    (Style::Old, _) => {
                        // 兩排沿街店屋，背靠背
                        let depth_total = v1 - v0;
                        let rows: &[(f32, bool)] = if depth_total > 30.0 { &[(0.0, false), (1.0, true)] } else { &[(0.0, false)] };
                        for &(row, back) in rows {
                            let d = if rows.len() == 2 { (depth_total * 0.5 - 1.0).min(22.0) } else { depth_total };
                            let vc = if back { v1 - d * 0.5 } else { v0 + d * 0.5 };
                            let mut uu = u0;
                            while uu < u1 - 5.0 {
                                let w = rng.range(7.0, 15.0).min(u1 - uu);
                                let town = s.tier == Tier::Town;
                                let center_boost = (1.0 - dist_c).clamp(0.0, 1.0);
                                let floors = if town {
                                    rng.range(2.0, 4.5)
                                } else if rng.f32() < 0.08 {
                                    rng.range(12.0, 20.0)
                                } else {
                                    rng.range(3.0, 7.0) + center_boost * rng.range(0.0, 8.0)
                                };
                                let kind = if town || s.kind != TownKind::City { BuildingKind::TownHouse } else { BuildingKind::OldShop };
                                let ht = floors.round().max(2.0) * 3.1 + 0.6;
                                let _ = row;
                                // back 列朝向另一側街道：以負深度標記
                                lots.push((uu + w * 0.5, vc, w - 0.4, if back { -d } else { d }, ht, kind));
                                uu += w;
                            }
                        }
                    }
                }
                for (lu, lv, w, d, ht, kind) in lots {
                    let flip = d < 0.0;
                    let d = d.abs();
                    let (wx, wz) = to_world(lu, lv);
                    // 四角都要在城內、不在水裡、不壓城際道路
                    let mut ok = true;
                    let mut ymin = f32::MAX;
                    for (cu, cv) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5), (0.0, 0.0)] {
                        let (px, pz) = to_world(lu + cu * w, lv + cv * d);
                        if !inside(px, pz) || wet(px, pz) {
                            ok = false;
                            break;
                        }
                        let (gx, gz) = pg.cell_of(px, pz);
                        if pg.road[gz * pg.n + gx] != 0 {
                            ok = false;
                            break;
                        }
                        ymin = ymin.min(h.sample(px, pz));
                    }
                    if !ok {
                        continue;
                    }
                    // 局部 +Z 朝向街道：前排朝 v0 方向（-v），後排朝 +v
                    let face = if flip { s.street_angle } else { s.street_angle + core::f32::consts::PI };
                    let angle = match kind {
                        BuildingKind::OldShop | BuildingKind::TownHouse | BuildingKind::Podium => face,
                        _ => s.street_angle,
                    };
                    buildings.push(Building {
                        x: wx,
                        y: ymin - 0.5,
                        z: wz,
                        angle,
                        sx: w,
                        sy: ht + 0.5,
                        sz: d,
                        kind,
                        seed: rng.next_u64() as u32,
                    });
                }
            }
        }
    }
    CityOut { streets, buildings }
}
