//! 宏觀地形：島嶼輪廓 + 丘陵 + 山脈，然後做水力侵蝕。

use crate::grid::Grid;
use crate::noise::{fbm, lerp, ridged, smoothstep};
use crate::rng::Rng;

/// 海岸場的陸地閾值：coast_field < COAST 為陸地
pub const COAST: f32 = 0.885;

pub fn coast_field(x: f32, z: f32, size: f32, seed: u32) -> f32 {
    let half = size * 0.5;
    let dx = (x - half) / half;
    let dz = (z - half) / half;
    let euclid = libm::sqrtf(dx * dx + dz * dz);
    let cheb = libm::fabsf(dx).max(libm::fabsf(dz));
    let d = lerp(euclid, cheb, 0.65);
    let s = size / 12288.0;
    d + 0.20 * fbm(x / (3200.0 * s), z / (3200.0 * s), seed ^ 0xC0A5, 4)
        + 0.06 * fbm(x / (700.0 * s), z / (700.0 * s), seed ^ 0xBEEF, 3)
}

pub fn base_height(x: f32, z: f32, size: f32, seed: u32) -> f32 {
    let s = (size / 12288.0).sqrt();
    // 域扭曲讓山脊自然彎曲
    let wx = x + 700.0 * fbm(x / 2600.0, z / 2600.0, seed ^ 0x11, 3);
    let wz = z + 700.0 * fbm(x / 2600.0 + 31.0, z / 2600.0 - 17.0, seed ^ 0x22, 3);

    let cf = coast_field(x, z, size, seed);
    // 有些海岸是懸崖：山脈可以一直延伸到海邊
    let cliff = smoothstep(0.25, 0.6, fbm(x / 2200.0, z / 2200.0, seed ^ 0x33, 2));
    let inland = smoothstep(COAST, lerp(0.40, 0.80, cliff), cf);

    let mountain_mask =
        smoothstep(0.48, 0.72, 0.5 + 0.5 * fbm(x / (5200.0 * s), z / (5200.0 * s), seed ^ 0x44, 3)) * inland;
    let plains = smoothstep(-0.25, 0.25, fbm(x / 4200.0, z / 4200.0, seed ^ 0x55, 2));
    let hills = 0.5 + 0.5 * fbm(wx / 1500.0, wz / 1500.0, seed ^ 0x66, 5);
    let ridge = ridged(wx / 2400.0, wz / 2400.0, seed ^ 0x77, 6);

    let land = 3.0
        + 55.0 * hills * lerp(0.15, 1.0, plains) * libm::sqrtf(inland)
        + mountain_mask * (ridge * 760.0 + 40.0 * hills);

    let shore = smoothstep(COAST + 0.01, COAST - 0.035, cf);
    let ocean = -3.0 - 45.0 * smoothstep(COAST, COAST + 0.12, cf);
    lerp(ocean, land, shore)
}

pub fn generate_base(n: usize, size: f32, seed: u32) -> Grid<f32> {
    let cell = size / n as f32;
    let mut g = Grid::new(n, cell, 0.0f32);
    for y in 0..n {
        for x in 0..n {
            let (wx, wz) = g.center(x, y);
            g.data[y * n + x] = base_height(wx, wz, size, seed);
        }
    }
    g
}

/// 水滴侵蝕（在「格單位」高度上運算，與解析度無關）
pub fn erode(h: &mut Grid<f32>, seed: u64, droplets: usize) {
    let n = h.n;
    let cell = h.cell;
    for v in h.data.iter_mut() {
        *v /= cell;
    }
    let radius: i32 = 3;
    let mut brush: Vec<(i32, i32, f32)> = Vec::new();
    let mut wsum = 0.0;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let d = libm::sqrtf((dx * dx + dy * dy) as f32);
            if d <= radius as f32 {
                let w = 1.0 - d / radius as f32;
                brush.push((dx, dy, w));
                wsum += w;
            }
        }
    }
    for b in brush.iter_mut() {
        b.2 /= wsum;
    }

    const INERTIA: f32 = 0.08;
    const CAPACITY: f32 = 3.0;
    const MIN_CAP: f32 = 0.01;
    const DEPOSIT: f32 = 0.25;
    const ERODE: f32 = 0.25;
    const EVAP: f32 = 0.02;
    const GRAVITY: f32 = 4.0;
    const LIFE: usize = 48;

    let mut rng = Rng::derive(seed, 0xE120);
    let nf = (n - 2) as f32;
    let hgrad = |d: &[f32], px: f32, py: f32| -> (f32, f32, f32) {
        let ix = px as usize;
        let iy = py as usize;
        let u = px - ix as f32;
        let v = py - iy as f32;
        let i = iy * n + ix;
        let a = d[i];
        let b = d[i + 1];
        let c = d[i + n];
        let e = d[i + n + 1];
        let gx = (b - a) * (1.0 - v) + (e - c) * v;
        let gy = (c - a) * (1.0 - u) + (e - b) * u;
        let hh = a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + c * (1.0 - u) * v + e * u * v;
        (hh, gx, gy)
    };

    for _ in 0..droplets {
        let mut px = rng.f32() * nf;
        let mut py = rng.f32() * nf;
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let mut speed = 1.0f32;
        let mut water = 1.0f32;
        let mut sed = 0.0f32;
        for _ in 0..LIFE {
            let ix = px as usize;
            let iy = py as usize;
            let u = px - ix as f32;
            let v = py - iy as f32;
            let (hh, gx, gy) = hgrad(&h.data, px, py);
            if hh < 0.0 {
                break; // 進入海洋
            }
            dx = dx * INERTIA - gx * (1.0 - INERTIA);
            dy = dy * INERTIA - gy * (1.0 - INERTIA);
            let len = libm::sqrtf(dx * dx + dy * dy);
            if len < 1e-6 {
                break;
            }
            dx /= len;
            dy /= len;
            let nx = px + dx;
            let ny = py + dy;
            if nx < 0.0 || ny < 0.0 || nx >= nf || ny >= nf {
                break;
            }
            let (nh, _, _) = hgrad(&h.data, nx, ny);
            let dh = nh - hh;
            let cap = (-dh * speed * water * CAPACITY).max(MIN_CAP);
            let i = iy * n + ix;
            if sed > cap || dh > 0.0 {
                let dep = if dh > 0.0 { dh.min(sed) } else { (sed - cap) * DEPOSIT };
                sed -= dep;
                h.data[i] += dep * (1.0 - u) * (1.0 - v);
                h.data[i + 1] += dep * u * (1.0 - v);
                h.data[i + n] += dep * (1.0 - u) * v;
                h.data[i + n + 1] += dep * u * v;
            } else {
                let amount = ((cap - sed) * ERODE).min(-dh);
                for &(bx, by, w) in &brush {
                    let x = ix as i32 + bx;
                    let y = iy as i32 + by;
                    if x >= 0 && y >= 0 && (x as usize) < n && (y as usize) < n {
                        let j = y as usize * n + x as usize;
                        let take = (amount * w).min(h.data[j].max(0.0));
                        h.data[j] -= take;
                        sed += take;
                    }
                }
            }
            speed = libm::sqrtf((speed * speed + dh * GRAVITY).max(0.0));
            water *= 1.0 - EVAP;
            px = nx;
            py = ny;
        }
    }
    for v in h.data.iter_mut() {
        *v *= cell;
    }
}
