//! 給遊戲使用的精細數據：渲染地形高度、碰撞高度（含橋面）、道路遮罩、水面、地表權重。

use crate::grid::Grid;
use crate::hydro::{Water, NO_WATER};
use crate::landcover::Cover;
use crate::noise::{fbm, smoothstep};
use crate::roads::{Road, RoadClass};
use crate::World;

pub struct FineData {
    pub n: usize,
    pub cell: f32,
    /// 渲染用地形（道路處已整平，不含橋）
    pub height: Vec<f32>,
    /// 物理用（含橋面）
    pub collide: Vec<f32>,
    /// 0..255，道路覆蓋度
    pub road: Vec<u8>,
    /// 宏觀解析度的水面高度；無水處低於地形
    pub water_n: usize,
    pub water: Vec<f32>,
    /// 宏觀解析度 RGBA：森林、岩石、沙、特殊（茶園 / 花田 / 城市）
    pub cover: Vec<u8>,
}

impl FineData {
    pub fn sample(&self, data: &[f32], x: f32, z: f32) -> f32 {
        let n = self.n as i32;
        let fx = x / self.cell - 0.5;
        let fz = z / self.cell - 0.5;
        let x0 = libm::floorf(fx);
        let z0 = libm::floorf(fz);
        let tx = fx - x0;
        let tz = fz - z0;
        let g = |x: i32, z: i32| data[(z.clamp(0, n - 1) * n + x.clamp(0, n - 1)) as usize];
        let (x0, z0) = (x0 as i32, z0 as i32);
        let a = g(x0, z0) + (g(x0 + 1, z0) - g(x0, z0)) * tx;
        let b = g(x0, z0 + 1) + (g(x0 + 1, z0 + 1) - g(x0, z0 + 1)) * tx;
        a + (b - a) * tz
    }

    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        self.sample(&self.collide, x, z)
    }

    pub fn road_at(&self, x: f32, z: f32) -> f32 {
        let (cx, cz) = ((x / self.cell) as i32, (z / self.cell) as i32);
        let n = self.n as i32;
        if cx < 0 || cz < 0 || cx >= n || cz >= n {
            return 0.0;
        }
        self.road[(cz * n + cx) as usize] as f32 / 255.0
    }
}

fn shoulder(c: RoadClass) -> f32 {
    match c {
        RoadClass::Highway => 14.0,
        RoadClass::Regional | RoadClass::Mountain => 10.0,
        _ => 3.0,
    }
}

pub fn build(world: &World, n: usize) -> FineData {
    let size = world.size;
    let cell = size / n as f32;
    let seed = world.seed as u32;
    let macro_h = &world.height;
    let mut height = vec![0.0f32; n * n];
    let mn = macro_h.n;
    for z in 0..n {
        for x in 0..n {
            let wx = (x as f32 + 0.5) * cell;
            let wz = (z as f32 + 0.5) * cell;
            let mut h = macro_h.sample(wx, wz);
            let (mx, mz) = macro_h.cell_of(wx, wz);
            let mut near_water = false;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    near_water |= world.hydro.kind.get_clamped(mx + dx, mz + dz) != Water::None as u8;
                }
            }
            let urban = world.urban.get_clamped(mx, mz) != 0;
            if !near_water && !urban {
                h += 1.2 * fbm(wx / 40.0, wz / 40.0, seed ^ 0xF1, 3);
            }
            height[z * n + x] = h;
        }
    }
    let mut collide = height.clone();
    let mut road = vec![0u8; n * n];
    let mut best_w = vec![0.0f32; n * n];
    let mut target = vec![0.0f32; n * n];

    let mut raster = |r: &Road| {
        let hw = r.width * 0.5;
        let sh = shoulder(r.class);
        let reach = hw + 1.5 + sh;
        for i in 0..r.pts.len().saturating_sub(1) {
            let a = r.pts[i];
            let b = r.pts[i + 1];
            let bridge = r.bridge[i] || r.bridge[i + 1];
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let len2 = (dx * dx + dz * dz).max(1e-6);
            let minx = ((a[0].min(b[0]) - reach) / cell).floor().max(0.0) as usize;
            let maxx = (((a[0].max(b[0]) + reach) / cell).ceil() as usize).min(n - 1);
            let minz = ((a[2].min(b[2]) - reach) / cell).floor().max(0.0) as usize;
            let maxz = (((a[2].max(b[2]) + reach) / cell).ceil() as usize).min(n - 1);
            for z in minz..=maxz {
                for x in minx..=maxx {
                    let px = (x as f32 + 0.5) * cell;
                    let pz = (z as f32 + 0.5) * cell;
                    let t = (((px - a[0]) * dx + (pz - a[2]) * dz) / len2).clamp(0.0, 1.0);
                    let qx = a[0] + dx * t;
                    let qz = a[2] + dz * t;
                    let d = libm::hypotf(px - qx, pz - qz);
                    if d > reach {
                        continue;
                    }
                    let y = a[1] + (b[1] - a[1]) * t;
                    let k = z * n + x;
                    let m = ((hw + 0.8 - d) / 1.6 + 0.5).clamp(0.0, 1.0);
                    if bridge {
                        if d < hw + 0.5 {
                            collide[k] = collide[k].max(y);
                        }
                        continue;
                    }
                    road[k] = road[k].max((m * 255.0) as u8);
                    let w = 1.0 - smoothstep(hw + 1.5, reach, d);
                    if w > best_w[k] {
                        best_w[k] = w;
                        target[k] = y - 0.12;
                    }
                }
            }
        }
    };
    for r in world.roads.iter().chain(world.streets.iter()) {
        raster(r);
    }
    for k in 0..n * n {
        let w = best_w[k];
        if w > 0.0 {
            let v = height[k] + (target[k] - height[k]) * w;
            height[k] = v;
            // 橋面已寫入 collide，保留較高者
            collide[k] = if collide[k] > v + 0.5 { collide[k] } else { v };
        }
    }

    let water_n = mn;
    let mut water = vec![0.0f32; mn * mn];
    let mut cover = vec![0u8; mn * mn * 4];
    for k in 0..mn * mn {
        let lv = world.hydro.level.data[k];
        water[k] = if lv > NO_WATER * 0.5 { lv } else { macro_h.data[k] - 4.0 };
        let c = world.cover.data[k];
        let o = k * 4;
        cover[o] = if c == Cover::Forest as u8 { 255 } else { 0 };
        cover[o + 1] = if c == Cover::Rock as u8 { 255 } else { 0 };
        cover[o + 2] = if c == Cover::Beach as u8 { 255 } else { 0 };
        cover[o + 3] = match c {
            x if x == Cover::Tea as u8 => 170,
            x if x == Cover::Flowers as u8 => 255,
            x if x == Cover::Urban as u8 => 85,
            _ => 0,
        };
    }
    let _ = Grid::<f32>::new(1, 1.0, 0.0);
    FineData { n, cell, height, collide, road, water_n, water, cover }
}
