//! 水系：海洋判定、窪地填充（湖泊）、流量累積、河流下切。

use crate::grid::Grid;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

pub const NO_WATER: f32 = -1.0e4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Water {
    None = 0,
    Ocean = 1,
    Lake = 2,
    River = 3,
}

pub struct Hydro {
    pub kind: Grid<u8>,
    /// 水面高度；沒有水的格子為 NO_WATER
    pub level: Grid<f32>,
    /// 流量累積（集水面積，m²）
    pub flow: Grid<f32>,
}

#[derive(PartialEq)]
struct Item(f32, u32);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        // 最小堆；相同高度以索引決勝，保證確定性
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

const N8: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

pub fn ocean_mask(h: &Grid<f32>) -> Vec<bool> {
    let n = h.n;
    let mut ocean = vec![false; n * n];
    let mut q = VecDeque::new();
    for i in 0..n {
        for &(x, y) in &[(i, 0), (i, n - 1), (0, i), (n - 1, i)] {
            let k = y * n + x;
            if h.data[k] < 0.0 && !ocean[k] {
                ocean[k] = true;
                q.push_back(k);
            }
        }
    }
    while let Some(k) = q.pop_front() {
        let (x, y) = ((k % n) as i32, (k / n) as i32);
        for &(dx, dy) in &N8[..4] {
            let (nx, ny) = (x + dx, y + dy);
            if h.in_bounds(nx, ny) {
                let j = ny as usize * n + nx as usize;
                if !ocean[j] && h.data[j] < 0.0 {
                    ocean[j] = true;
                    q.push_back(j);
                }
            }
        }
    }
    ocean
}

pub fn build(h: &mut Grid<f32>, river_catchment_m2: f32, max_lake_m2: f32) -> Hydro {
    let n = h.n;
    let cell = h.cell;
    let ocean = ocean_mask(h);

    // 1. Priority-Flood 填窪
    let eps = 1.0e-3;
    let mut filled = h.data.clone();
    let mut done = vec![false; n * n];
    let mut heap = BinaryHeap::new();
    for k in 0..n * n {
        let (x, y) = (k % n, k / n);
        let border = x == 0 || y == 0 || x == n - 1 || y == n - 1;
        if ocean[k] || border {
            done[k] = true;
            heap.push(Item(filled[k], k as u32));
        }
    }
    while let Some(Item(fh, k)) = heap.pop() {
        let k = k as usize;
        let (x, y) = ((k % n) as i32, (k / n) as i32);
        for &(dx, dy) in &N8 {
            let (nx, ny) = (x + dx, y + dy);
            if !h.in_bounds(nx, ny) {
                continue;
            }
            let j = ny as usize * n + nx as usize;
            if done[j] {
                continue;
            }
            done[j] = true;
            filled[j] = h.data[j].max(fh + eps);
            heap.push(Item(filled[j], j as u32));
        }
    }

    // 2. 湖泊：填充深度 > 1.5 m 的連通區；太小的直接填平
    let mut kind = Grid::new(n, cell, Water::None as u8);
    let mut level = Grid::new(n, cell, NO_WATER);
    for k in 0..n * n {
        if ocean[k] {
            kind.data[k] = Water::Ocean as u8;
            level.data[k] = 0.0;
        }
    }
    let mut seen = vec![false; n * n];
    let mut lakebed = vec![false; n * n];
    for k in 0..n * n {
        if seen[k] || ocean[k] || filled[k] - h.data[k] < 1.5 {
            continue;
        }
        let mut comp = vec![k];
        seen[k] = true;
        let mut qi = 0;
        while qi < comp.len() {
            let c = comp[qi];
            qi += 1;
            let (x, y) = ((c % n) as i32, (c / n) as i32);
            for &(dx, dy) in &N8[..4] {
                let (nx, ny) = (x + dx, y + dy);
                if h.in_bounds(nx, ny) {
                    let j = ny as usize * n + nx as usize;
                    if !seen[j] && !ocean[j] && filled[j] - h.data[j] > 0.3 {
                        seen[j] = true;
                        comp.push(j);
                    }
                }
            }
        }
        if comp.len() < 40 {
            for &c in &comp {
                h.data[c] = filled[c];
            }
        } else {
            for &c in &comp {
                lakebed[c] = true;
            }
            // 湖面積上限：降低水位直到面積不超過上限，露出的湖床變成陸地
            let max_cells = (max_lake_m2 / (cell * cell)) as usize;
            let mut lvl = comp.iter().map(|&c| filled[c]).fold(f32::MIN, f32::max);
            if comp.len() > max_cells {
                let mut hs: Vec<f32> = comp.iter().map(|&c| h.data[c]).collect();
                hs.sort_by(|a, b| a.total_cmp(b));
                lvl = lvl.min(hs[max_cells.max(1) - 1]);
            }
            for &c in &comp {
                if h.data[c] < lvl - 0.2 {
                    kind.data[c] = Water::Lake as u8;
                    level.data[c] = lvl;
                }
            }
        }
    }

    // 3. 流向與流量累積（在填充後的地形上，D8 最陡下降）
    let mut order: Vec<u32> = (0..(n * n) as u32).collect();
    order.sort_by(|&a, &b| filled[b as usize].total_cmp(&filled[a as usize]).then(a.cmp(&b)));
    let mut recv = vec![u32::MAX; n * n];
    for k in 0..n * n {
        if ocean[k] {
            continue;
        }
        let (x, y) = ((k % n) as i32, (k / n) as i32);
        let mut best = 0.0f32;
        for &(dx, dy) in &N8 {
            let (nx, ny) = (x + dx, y + dy);
            if !h.in_bounds(nx, ny) {
                continue;
            }
            let j = ny as usize * n + nx as usize;
            let dist = if dx != 0 && dy != 0 { 1.4142 } else { 1.0 };
            let drop = (filled[k] - filled[j]) / dist;
            if drop > best {
                best = drop;
                recv[k] = j as u32;
            }
        }
    }
    let ca = cell * cell;
    let mut flow = Grid::new(n, cell, ca);
    for &k in &order {
        let r = recv[k as usize];
        if r != u32::MAX {
            let f = flow.data[k as usize];
            flow.data[r as usize] += f;
        }
    }

    // 4. 河流下切
    for k in 0..n * n {
        let a = flow.data[k];
        if a < river_catchment_m2 || ocean[k] || lakebed[k] {
            continue;
        }
        let width = (6.0 + 0.012 * libm::sqrtf(a)).min(60.0);
        let depth = 1.5 + width * 0.05;
        let bed = filled[k] - depth;
        let surf = filled[k] - depth * 0.3;
        let r = (width * 0.5 / cell).max(0.7);
        let ri = libm::ceilf(r + 1.0) as i32;
        let (x, y) = ((k % n) as i32, (k / n) as i32);
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let (nx, ny) = (x + dx, y + dy);
                if !h.in_bounds(nx, ny) {
                    continue;
                }
                let j = ny as usize * n + nx as usize;
                if ocean[j] {
                    continue;
                }
                let d = libm::sqrtf((dx * dx + dy * dy) as f32) / (r + 1.0);
                if d > 1.0 {
                    continue;
                }
                let carved = bed + d * d * (depth + 1.0);
                if carved < h.data[j] {
                    h.data[j] = carved;
                }
                if d * (r + 1.0) <= r && kind.data[j] != Water::Lake as u8 {
                    kind.data[j] = Water::River as u8;
                    if surf > level.data[j] {
                        level.data[j] = surf;
                    }
                }
            }
        }
    }

    Hydro { kind, level, flow }
}
