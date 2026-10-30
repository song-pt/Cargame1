//! 2D 梯度噪聲與分形組合。只使用加減乘與 floor，跨平台位元一致。

use crate::rng::hash3;

const GRADS: [(f32, f32); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (0.70710677, 0.70710677),
    (-0.70710677, 0.70710677),
    (0.70710677, -0.70710677),
    (-0.70710677, -0.70710677),
];

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad(ix: i32, iy: i32, seed: u32, dx: f32, dy: f32) -> f32 {
    let g = GRADS[(hash3(ix, iy, seed) & 7) as usize];
    g.0 * dx + g.1 * dy
}

/// 約 [-1, 1]
pub fn perlin(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = libm::floorf(x);
    let y0 = libm::floorf(y);
    let ix = x0 as i32;
    let iy = y0 as i32;
    let fx = x - x0;
    let fy = y - y0;
    let u = fade(fx);
    let v = fade(fy);
    let n00 = grad(ix, iy, seed, fx, fy);
    let n10 = grad(ix + 1, iy, seed, fx - 1.0, fy);
    let n01 = grad(ix, iy + 1, seed, fx, fy - 1.0);
    let n11 = grad(ix + 1, iy + 1, seed, fx - 1.0, fy - 1.0);
    let a = n00 + u * (n10 - n00);
    let b = n01 + u * (n11 - n01);
    (a + v * (b - a)) * 1.414
}

/// 分形布朗運動，輸出約 [-1, 1]
pub fn fbm(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let mut fx = x;
    let mut fy = y;
    for o in 0..octaves {
        sum += amp * perlin(fx, fy, seed.wrapping_add(o * 1013));
        norm += amp;
        amp *= 0.5;
        // 旋轉以減少網格方向性
        let nx = fx * 1.6 - fy * 1.2;
        let ny = fx * 1.2 + fy * 1.6;
        fx = nx + 17.3;
        fy = ny - 9.1;
    }
    sum / norm
}

/// 山脊多重分形，輸出約 [0, 1]
pub fn ridged(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut weight = 1.0;
    let mut norm = 0.0;
    let mut fx = x;
    let mut fy = y;
    for o in 0..octaves {
        let n = 1.0 - libm::fabsf(perlin(fx, fy, seed.wrapping_add(o * 7919)));
        let n = n * n * weight;
        weight = (n * 2.0).clamp(0.0, 1.0);
        sum += n * amp;
        norm += amp;
        amp *= 0.5;
        let nx = fx * 1.6 - fy * 1.2;
        let ny = fx * 1.2 + fy * 1.6;
        fx = nx + 3.7;
        fy = ny + 11.9;
    }
    sum / norm
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
