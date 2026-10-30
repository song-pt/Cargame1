//! CDLOD 地形四叉樹選擇 + 高度最小/最大值金字塔。

use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct NodeInstance {
    pub node: [f32; 4],
    pub morph: [f32; 4],
}

pub struct MinMax {
    /// levels[k] 的格距為 base_cell * 2^k
    levels: Vec<(usize, Vec<f32>, Vec<f32>)>,
    base_cell: f32,
}

impl MinMax {
    pub fn new(n: usize, cell: f32, data: &[f32]) -> Self {
        let mut levels = Vec::new();
        let mut mn = vec![0.0f32; n * n];
        let mut mx = vec![0.0f32; n * n];
        for z in 0..n {
            for x in 0..n {
                let mut lo = f32::MAX;
                let mut hi = f32::MIN;
                for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let v = data[(z + dz).min(n - 1) * n + (x + dx).min(n - 1)];
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                mn[z * n + x] = lo;
                mx[z * n + x] = hi;
            }
        }
        levels.push((n, mn, mx));
        let mut cur = n;
        while cur > 1 {
            let next = cur / 2;
            let (_, pmn, pmx) = levels.last().unwrap();
            let mut mn = vec![0.0f32; next * next];
            let mut mx = vec![0.0f32; next * next];
            for z in 0..next {
                for x in 0..next {
                    let mut lo = f32::MAX;
                    let mut hi = f32::MIN;
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let k = (z * 2 + dz) * cur + x * 2 + dx;
                        lo = lo.min(pmn[k]);
                        hi = hi.max(pmx[k]);
                    }
                    mn[z * next + x] = lo;
                    mx[z * next + x] = hi;
                }
            }
            levels.push((next, mn, mx));
            cur = next;
        }
        Self { levels, base_cell: cell }
    }

    pub fn range(&self, x0: f32, z0: f32, size: f32) -> (f32, f32) {
        let mut k = 0;
        while k + 1 < self.levels.len() && self.base_cell * (1 << k) as f32 * 0.5 < size {
            k += 1;
        }
        let (n, mn, mx) = &self.levels[k];
        let c = self.base_cell * (1 << k) as f32;
        let n = *n as i32;
        let ix0 = ((x0 / c) as i32 - 1).clamp(0, n - 1);
        let iz0 = ((z0 / c) as i32 - 1).clamp(0, n - 1);
        let ix1 = (((x0 + size) / c) as i32 + 1).clamp(0, n - 1);
        let iz1 = (((z0 + size) / c) as i32 + 1).clamp(0, n - 1);
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for z in iz0..=iz1 {
            for x in ix0..=ix1 {
                let i = (z * n + x) as usize;
                lo = lo.min(mn[i]);
                hi = hi.max(mx[i]);
            }
        }
        (lo, hi)
    }
}

pub struct Lod {
    pub root: f32,
    pub world: f32,
    pub levels: u32,
    pub ranges: Vec<f32>,
}

impl Lod {
    pub fn new(world: f32, leaf: f32, range_factor: f32) -> Self {
        let mut root = leaf;
        let mut levels = 0;
        while root < world {
            root *= 2.0;
            levels += 1;
        }
        let ranges = (0..=levels).map(|l| leaf * (1u32 << l) as f32 * range_factor).collect();
        Self { root, world, levels, ranges }
    }

    pub fn select(&self, cam: Vec3, planes: &[Vec4; 6], mm: &MinMax, wet: Option<&MinMax>, out: &mut Vec<NodeInstance>) {
        self.sel(0.0, 0.0, self.root, self.levels, cam, planes, mm, wet, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn sel(&self, x: f32, z: f32, size: f32, lod: u32, cam: Vec3, planes: &[Vec4; 6], mm: &MinMax, wet: Option<&MinMax>, out: &mut Vec<NodeInstance>) {
        if x >= self.world || z >= self.world {
            return;
        }
        if let Some(w) = wet {
            if w.range(x, z, size).1 <= 0.0 {
                return;
            }
        }
        let (lo, hi) = mm.range(x, z, size);
        let bmin = Vec3::new(x, lo - 2.0, z) - cam;
        let bmax = Vec3::new(x + size, hi + 2.0, z + size) - cam;
        if !aabb_in_frustum(bmin, bmax, planes) {
            return;
        }
        let near = lod > 0 && dist_to_aabb(bmin, bmax) < self.ranges[(lod - 1) as usize];
        if !near {
            let end = self.ranges[lod as usize];
            let start = end * 0.72;
            out.push(NodeInstance { node: [x, z, size, lod as f32], morph: [start, end, 0.0, 0.0] });
            return;
        }
        let h = size * 0.5;
        for (cx, cz) in [(x, z), (x + h, z), (x, z + h), (x + h, z + h)] {
            self.sel(cx, cz, h, lod - 1, cam, planes, mm, wet, out);
        }
    }
}

fn dist_to_aabb(bmin: Vec3, bmax: Vec3) -> f32 {
    let d = bmin.max(-bmax).max(Vec3::ZERO);
    d.length()
}

fn aabb_in_frustum(bmin: Vec3, bmax: Vec3, planes: &[Vec4; 6]) -> bool {
    for p in planes {
        let v = Vec3::new(if p.x > 0.0 { bmax.x } else { bmin.x }, if p.y > 0.0 { bmax.y } else { bmin.y }, if p.z > 0.0 { bmax.z } else { bmin.z });
        if p.x * v.x + p.y * v.y + p.z * v.z + p.w < 0.0 {
            return false;
        }
    }
    true
}

/// 從 view_proj（WebGPU 深度 0..1，反向 Z）取得視錐平面
pub fn frustum_planes(m: glam::Mat4) -> [Vec4; 6] {
    let r0 = m.row(0);
    let r1 = m.row(1);
    let r2 = m.row(2);
    let r3 = m.row(3);
    let norm = |p: Vec4| p / p.truncate().length().max(1e-6);
    [
        norm(r3 + r0),
        norm(r3 - r0),
        norm(r3 + r1),
        norm(r3 - r1),
        norm(r3 - r2), // 反向 Z：近平面 z <= w
        norm(r2),      // 遠平面 z >= 0（無限遠時為退化平面，總是通過）
    ]
}
