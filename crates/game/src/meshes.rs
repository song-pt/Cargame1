//! CPU 端幾何：地形補丁網格、單位立方體、圓柱、道路帶狀網格。

use bytemuck::{Pod, Zeroable};
use worldgen::roads::Road;

pub const PATCH: u32 = 32;

pub fn patch_grid() -> (Vec<[f32; 2]>, Vec<u16>) {
    let n = PATCH + 1;
    let mut v = Vec::with_capacity((n * n) as usize);
    for z in 0..n {
        for x in 0..n {
            v.push([x as f32, z as f32]);
        }
    }
    let mut idx = Vec::with_capacity((PATCH * PATCH * 6) as usize);
    for z in 0..PATCH {
        for x in 0..PATCH {
            let a = (z * n + x) as u16;
            let b = a + 1;
            let c = a + n as u16;
            let d = c + 1;
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    (v, idx)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

fn face(out: &mut Vec<MeshVertex>, idx: &mut Vec<u16>, c: [f32; 3], u: [f32; 3], v: [f32; 3], n: [f32; 3]) {
    let base = out.len() as u16;
    for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        out.push(MeshVertex {
            pos: [c[0] + u[0] * su + v[0] * sv, c[1] + u[1] * su + v[1] * sv, c[2] + u[2] * su + v[2] * sv],
            normal: n,
        });
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// 單位立方體。`y0` 為底面高度（建築用 0..1，車輛零件用 -0.5..0.5）
pub fn cube(y0: f32) -> (Vec<MeshVertex>, Vec<u16>) {
    let mut v = Vec::new();
    let mut i = Vec::new();
    let cy = y0 + 0.5;
    // 每個面：u × v = n（從外面看是逆時針）
    face(&mut v, &mut i, [0.5, cy, 0.0], [0.0, 0.0, -0.5], [0.0, 0.5, 0.0], [1.0, 0.0, 0.0]);
    face(&mut v, &mut i, [-0.5, cy, 0.0], [0.0, 0.0, 0.5], [0.0, 0.5, 0.0], [-1.0, 0.0, 0.0]);
    face(&mut v, &mut i, [0.0, cy + 0.5, 0.0], [0.0, 0.0, 0.5], [0.5, 0.0, 0.0], [0.0, 1.0, 0.0]);
    face(&mut v, &mut i, [0.0, cy - 0.5, 0.0], [0.5, 0.0, 0.0], [0.0, 0.0, 0.5], [0.0, -1.0, 0.0]);
    face(&mut v, &mut i, [0.0, cy, 0.5], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 1.0]);
    face(&mut v, &mut i, [0.0, cy, -0.5], [-0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, -1.0]);
    (v, i)
}

/// 沿 X 軸的圓柱，半徑 0.5，長度 1
pub fn cylinder(seg: u32) -> (Vec<MeshVertex>, Vec<u16>) {
    let mut v = Vec::new();
    let mut idx = Vec::new();
    for s in 0..=seg {
        let a = s as f32 / seg as f32 * core::f32::consts::TAU;
        let (y, z) = (a.cos(), a.sin());
        for x in [-0.5f32, 0.5] {
            v.push(MeshVertex { pos: [x, y * 0.5, z * 0.5], normal: [0.0, y, z] });
        }
    }
    for s in 0..seg {
        let a = (s * 2) as u16;
        idx.extend_from_slice(&[a, a + 2, a + 1, a + 1, a + 2, a + 3]);
    }
    for (x, nx) in [(-0.5f32, -1.0f32), (0.5, 1.0)] {
        let c = v.len() as u16;
        v.push(MeshVertex { pos: [x, 0.0, 0.0], normal: [nx, 0.0, 0.0] });
        for s in 0..=seg {
            let a = s as f32 / seg as f32 * core::f32::consts::TAU;
            v.push(MeshVertex { pos: [x, a.cos() * 0.5, a.sin() * 0.5], normal: [nx, 0.0, 0.0] });
        }
        for s in 0..seg as u16 {
            if nx > 0.0 {
                idx.extend_from_slice(&[c, c + 1 + s, c + 2 + s]);
            } else {
                idx.extend_from_slice(&[c, c + 2 + s, c + 1 + s]);
            }
        }
    }
    (v, idx)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RoadVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub info: [f32; 2],
}

pub fn road_mesh<'a>(roads: impl Iterator<Item = &'a Road>) -> (Vec<RoadVertex>, Vec<u32>) {
    let mut v: Vec<RoadVertex> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for r in roads {
        let m = r.pts.len();
        if m < 2 {
            continue;
        }
        let hw = r.width * 0.5;
        let class = r.class.id() as f32;
        let base = v.len() as u32;
        let mut along = 0.0f32;
        let mut lefts = Vec::with_capacity(m);
        for i in 0..m {
            let a = r.pts[i.saturating_sub(1)];
            let b = r.pts[(i + 1).min(m - 1)];
            let (tx, tz) = (b[0] - a[0], b[2] - a[2]);
            let l = (tx * tx + tz * tz).sqrt().max(1e-4);
            let (nx, nz) = (-tz / l, tx / l);
            if i > 0 {
                let p = r.pts[i - 1];
                along += ((r.pts[i][0] - p[0]).powi(2) + (r.pts[i][2] - p[2]).powi(2)).sqrt();
            }
            let p = r.pts[i];
            let y = p[1] + 0.02;
            let left = [p[0] + nx * hw, y, p[2] + nz * hw];
            let right = [p[0] - nx * hw, y, p[2] - nz * hw];
            v.push(RoadVertex { pos: left, uv: [-1.0, along], info: [class, 0.0] });
            v.push(RoadVertex { pos: right, uv: [1.0, along], info: [class, 0.0] });
            lefts.push((left, right));
        }
        for i in 0..m as u32 - 1 {
            let a = base + i * 2;
            idx.extend_from_slice(&[a, a + 2, a + 1, a + 1, a + 2, a + 3]);
        }
        // 橋：兩側護欄面與底面
        for i in 0..m - 1 {
            if !(r.bridge[i] && r.bridge[i + 1]) {
                continue;
            }
            let (l0, r0) = lefts[i];
            let (l1, r1) = lefts[i + 1];
            for (p0, p1) in [(l0, l1), (r1, r0)] {
                let b = v.len() as u32;
                let down = 1.6;
                let up = 0.9;
                v.push(RoadVertex { pos: [p0[0], p0[1] + up, p0[2]], uv: [0.0, 0.0], info: [class, 1.0] });
                v.push(RoadVertex { pos: [p1[0], p1[1] + up, p1[2]], uv: [0.0, 0.0], info: [class, 1.0] });
                v.push(RoadVertex { pos: [p0[0], p0[1] - down, p0[2]], uv: [0.0, 0.0], info: [class, 1.0] });
                v.push(RoadVertex { pos: [p1[0], p1[1] - down, p1[2]], uv: [0.0, 0.0], info: [class, 1.0] });
                idx.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
            }
            let b = v.len() as u32;
            for p in [l0, r0, l1, r1] {
                v.push(RoadVertex { pos: [p[0], p[1] - 1.6, p[2]], uv: [0.0, 0.0], info: [class, 1.0] });
            }
            idx.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
        }
    }
    (v, idx)
}
