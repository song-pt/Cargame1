//! 城際路網：最小生成樹 + 補充邊 + 山道；地形代價 A* 尋路，平滑後生成縱斷面。

use crate::grid::Grid;
use crate::hydro::{Hydro, Water};
use crate::settle::{Settlement, Tier};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoadClass {
    Highway,
    Regional,
    Mountain,
    Arterial,
    Street,
    Alley,
}

impl RoadClass {
    pub fn width(self) -> f32 {
        match self {
            RoadClass::Highway => 14.0,
            RoadClass::Regional => 9.0,
            RoadClass::Mountain => 7.0,
            RoadClass::Arterial => 22.0,
            RoadClass::Street => 8.0,
            RoadClass::Alley => 4.5,
        }
    }
    pub fn max_grade(self) -> f32 {
        match self {
            RoadClass::Highway => 0.08,
            RoadClass::Regional => 0.10,
            RoadClass::Mountain => 0.14,
            _ => 0.16,
        }
    }
    pub fn id(self) -> u32 {
        self as u32
    }
}

#[derive(Clone, Debug)]
pub struct Road {
    pub class: RoadClass,
    pub width: f32,
    /// (x, y, z)，y 為路面高度
    pub pts: Vec<[f32; 3]>,
    pub bridge: Vec<bool>,
}

/// 尋路用的粗網格（宏觀網格的一半解析度）
pub struct PathGrid {
    pub n: usize,
    pub cell: f32,
    pub h: Vec<f32>,
    pub water: Vec<u8>,
    pub urban: Vec<bool>,
    pub coast: Vec<bool>,
    /// 0 = 無路，否則 RoadClass + 1
    pub road: Vec<u8>,
}

impl PathGrid {
    pub fn new(h: &Grid<f32>, hy: &Hydro, urban: &Grid<u8>) -> Self {
        let n = h.n / 2;
        let cell = h.cell * 2.0;
        let mut pg = PathGrid {
            n,
            cell,
            h: vec![0.0; n * n],
            water: vec![0; n * n],
            urban: vec![false; n * n],
            coast: vec![false; n * n],
            road: vec![0; n * n],
        };
        for y in 0..n {
            for x in 0..n {
                let mut hs = 0.0;
                let mut w = 0u8;
                let mut u = false;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (fx, fy) = (2 * x + dx, 2 * y + dy);
                    hs += h.get(fx, fy);
                    let k = hy.kind.get(fx, fy);
                    if k == Water::Ocean as u8 {
                        w = Water::Ocean as u8;
                    } else if k != 0 && w == 0 {
                        w = k;
                    }
                    u |= urban.get(fx, fy) > 0;
                }
                let k = y * n + x;
                pg.h[k] = hs * 0.25;
                pg.water[k] = w;
                pg.urban[k] = u;
            }
        }
        let d_ocean = crate::settle::distance_field(n, |k| pg.water[k] == Water::Ocean as u8);
        for k in 0..n * n {
            pg.coast[k] = pg.water[k] == 0 && (d_ocean[k] as f32) * cell < 320.0;
        }
        pg
    }

    pub fn cell_of(&self, x: f32, z: f32) -> (usize, usize) {
        let m = self.n as i32 - 1;
        (
            ((x / self.cell) as i32).clamp(0, m) as usize,
            ((z / self.cell) as i32).clamp(0, m) as usize,
        )
    }
}

#[derive(PartialEq)]
struct Node(f32, u32);
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

const MOVES: [(i32, i32); 16] = [
    (1, 0), (-1, 0), (0, 1), (0, -1),
    (1, 1), (-1, 1), (1, -1), (-1, -1),
    (2, 1), (2, -1), (-2, 1), (-2, -1),
    (1, 2), (1, -2), (-1, 2), (-1, -2),
];

pub struct Astar {
    g: Vec<f32>,
    came: Vec<u32>,
    stamp: Vec<u32>,
    gen: u32,
}

impl Astar {
    pub fn new(n: usize) -> Self {
        Self { g: vec![0.0; n * n], came: vec![0; n * n], stamp: vec![0; n * n], gen: 0 }
    }

    pub fn find(&mut self, pg: &PathGrid, start: usize, goal: usize, class: RoadClass, grade_relax: f32) -> Option<Vec<usize>> {
        let n = pg.n;
        self.gen += 1;
        let gen = self.gen;
        let max_grade = class.max_grade() * grade_relax;
        let (gx, gy) = ((goal % n) as f32, (goal / n) as f32);
        let heur = |k: usize| -> f32 {
            let (x, y) = ((k % n) as f32, (k / n) as f32);
            libm::hypotf(x - gx, y - gy) * pg.cell * 0.42
        };
        let mut open = BinaryHeap::new();
        self.g[start] = 0.0;
        self.stamp[start] = gen;
        self.came[start] = u32::MAX;
        open.push(Node(heur(start), start as u32));
        let urban_ok = class != RoadClass::Mountain;
        while let Some(Node(_, k)) = open.pop() {
            let k = k as usize;
            if k == goal {
                let mut path = vec![k];
                let mut c = k;
                while self.came[c] != u32::MAX {
                    c = self.came[c] as usize;
                    path.push(c);
                }
                path.reverse();
                return Some(path);
            }
            let gk = self.g[k];
            let (x, y) = ((k % n) as i32, (k / n) as i32);
            for &(dx, dy) in &MOVES {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                    continue;
                }
                let t = ny as usize * n + nx as usize;
                if pg.water[t] == Water::Ocean as u8 {
                    continue;
                }
                if dx.abs() + dy.abs() == 3 {
                    let m = (y + dy / 2) as usize * n + (x + dx / 2) as usize;
                    if pg.water[m] == Water::Ocean as u8 {
                        continue;
                    }
                }
                let dist = libm::sqrtf((dx * dx + dy * dy) as f32) * pg.cell;
                let wet = pg.water[t] != 0 || pg.water[k] != 0;
                let grade = if wet { 0.0 } else { libm::fabsf(pg.h[t] - pg.h[k]) / dist };
                if grade > max_grade {
                    continue;
                }
                let gr = grade / max_grade;
                let mut c = dist * (1.0 + 3.0 * gr * gr);
                if wet {
                    c *= 7.0;
                }
                if pg.road[t] != 0 {
                    c *= 0.45;
                } else if urban_ok && pg.urban[t] {
                    c *= 0.85;
                }
                if pg.coast[t] {
                    c *= 0.8;
                }
                if class == RoadClass::Mountain {
                    // 山道偏好高處，視野更好
                    c *= 1.0 - (pg.h[t] / 1200.0).clamp(0.0, 0.3);
                }
                let ng = gk + c;
                if self.stamp[t] != gen || ng < self.g[t] {
                    self.stamp[t] = gen;
                    self.g[t] = ng;
                    self.came[t] = k as u32;
                    open.push(Node(ng + heur(t), t as u32));
                }
            }
        }
        None
    }
}

fn dp_simplify(pts: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (ax, az) = (pts[a][0], pts[a][1]);
        let (bx, bz) = (pts[b][0], pts[b][1]);
        let (dx, dz) = (bx - ax, bz - az);
        let len = libm::hypotf(dx, dz).max(1e-6);
        let mut far = (0.0, 0);
        for i in a + 1..b {
            let d = libm::fabsf((pts[i][0] - ax) * dz - (pts[i][1] - az) * dx) / len;
            if d > far.0 {
                far = (d, i);
            }
        }
        if far.0 > tol {
            keep[far.1] = true;
            stack.push((a, far.1));
            stack.push((far.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn chaikin(pts: &[[f32; 2]], iters: usize) -> Vec<[f32; 2]> {
    let mut p = pts.to_vec();
    for _ in 0..iters {
        if p.len() < 3 {
            break;
        }
        let mut q = vec![p[0]];
        for w in p.windows(2) {
            let (a, b) = (w[0], w[1]);
            q.push([a[0] * 0.75 + b[0] * 0.25, a[1] * 0.75 + b[1] * 0.25]);
            q.push([a[0] * 0.25 + b[0] * 0.75, a[1] * 0.25 + b[1] * 0.75]);
        }
        q.push(*p.last().unwrap());
        p = q;
    }
    p
}

pub fn resample(pts: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
    if pts.len() < 2 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    let mut carry = 0.0;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = libm::hypotf(b[0] - a[0], b[1] - a[1]);
        let mut t = step - carry;
        while t < len {
            let f = t / len;
            out.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
            t += step;
        }
        carry = len - (t - step);
    }
    let last = *pts.last().unwrap();
    let prev = *out.last().unwrap();
    if libm::hypotf(last[0] - prev[0], last[1] - prev[1]) > step * 0.3 {
        out.push(last);
    } else {
        *out.last_mut().unwrap() = last;
    }
    out
}

/// 由平面折線建立帶縱斷面的道路
pub fn build_profile(
    xz: &[[f32; 2]],
    class: RoadClass,
    h: &Grid<f32>,
    hy: &Hydro,
    pin_start: Option<f32>,
    pin_end: Option<f32>,
) -> Road {
    let m = xz.len();
    let mut y: Vec<f32> = Vec::with_capacity(m);
    let mut bridge = Vec::with_capacity(m);
    for p in xz {
        let (cx, cz) = h.cell_of(p[0], p[1]);
        let k = hy.kind.get_clamped(cx, cz);
        let wet = k == Water::River as u8 || k == Water::Lake as u8 || k == Water::Ocean as u8;
        bridge.push(wet);
        y.push(h.sample(p[0], p[1]).max(0.6));
    }
    // 橋：在兩岸之間線性插值，並保證高於水面
    let mut i = 0;
    while i < m {
        if bridge[i] {
            let a = i.saturating_sub(1);
            let mut b = i;
            while b < m && bridge[b] {
                b += 1;
            }
            let b = b.min(m - 1);
            let (ya, yb) = (y[a], y[b]);
            let mut wl: f32 = 0.0;
            for j in a..=b {
                let (cx, cz) = h.cell_of(xz[j][0], xz[j][1]);
                wl = wl.max(hy.level.get_clamped(cx, cz));
            }
            for j in a..=b {
                let t = if b > a { (j - a) as f32 / (b - a) as f32 } else { 0.0 };
                y[j] = (ya + (yb - ya) * t).max(wl + 4.0);
            }
            // 引橋段也標為橋，避免地形整平侵入河道
            for j in a..=b {
                bridge[j] = true;
            }
            i = b + 1;
        } else {
            i += 1;
        }
    }
    let win = match class {
        RoadClass::Highway => 9,
        RoadClass::Regional => 6,
        RoadClass::Mountain => 4,
        _ => 3,
    };
    for _ in 0..2 {
        let src = y.clone();
        for j in 0..m {
            let lo = j.saturating_sub(win);
            let hi = (j + win).min(m - 1);
            let mut s = 0.0;
            for v in &src[lo..=hi] {
                s += v;
            }
            let avg = s / (hi - lo + 1) as f32;
            y[j] = if bridge[j] { src[j].max(avg) } else { avg };
        }
    }
    if let Some(p) = pin_start {
        let k = 12.min(m);
        for j in 0..k {
            let t = j as f32 / k as f32;
            y[j] = p + (y[j] - p) * t;
        }
    }
    if let Some(p) = pin_end {
        let k = 12.min(m);
        for j in 0..k {
            let t = j as f32 / k as f32;
            let idx = m - 1 - j;
            y[idx] = p + (y[idx] - p) * t;
        }
    }
    // 坡度限制（雙向）
    let g = class.max_grade();
    for _ in 0..2 {
        for j in 1..m {
            let d = libm::hypotf(xz[j][0] - xz[j - 1][0], xz[j][1] - xz[j - 1][1]);
            y[j] = y[j].clamp(y[j - 1] - g * d, y[j - 1] + g * d);
        }
        for j in (0..m.saturating_sub(1)).rev() {
            let d = libm::hypotf(xz[j][0] - xz[j + 1][0], xz[j][1] - xz[j + 1][1]);
            y[j] = y[j].clamp(y[j + 1] - g * d, y[j + 1] + g * d);
        }
    }
    Road {
        class,
        width: class.width(),
        pts: xz.iter().zip(&y).map(|(p, &yy)| [p[0], yy, p[1]]).collect(),
        bridge,
    }
}

/// 道路點空間索引，用於路口吸附
pub struct PointIndex {
    bucket: f32,
    n: usize,
    cells: Vec<Vec<(u32, u32)>>,
}

impl PointIndex {
    pub fn new(size: f32) -> Self {
        let bucket = 64.0;
        let n = (size / bucket) as usize + 1;
        Self { bucket, n, cells: vec![Vec::new(); n * n] }
    }
    pub fn add(&mut self, road: u32, roads: &[Road]) {
        for (i, p) in roads[road as usize].pts.iter().enumerate() {
            let k = self.key(p[0], p[2]);
            self.cells[k].push((road, i as u32));
        }
    }
    fn key(&self, x: f32, z: f32) -> usize {
        let m = self.n as i32 - 1;
        let bx = ((x / self.bucket) as i32).clamp(0, m) as usize;
        let bz = ((z / self.bucket) as i32).clamp(0, m) as usize;
        bz * self.n + bx
    }
    pub fn nearest(&self, x: f32, z: f32, roads: &[Road], max_d: f32) -> Option<[f32; 3]> {
        let mut best: Option<(f32, [f32; 3])> = None;
        let m = self.n as i32 - 1;
        let bx = (x / self.bucket) as i32;
        let bz = (z / self.bucket) as i32;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (cx, cz) = (bx + dx, bz + dz);
                if cx < 0 || cz < 0 || cx > m || cz > m {
                    continue;
                }
                for &(r, i) in &self.cells[cz as usize * self.n + cx as usize] {
                    let p = roads[r as usize].pts[i as usize];
                    let d = libm::hypotf(p[0] - x, p[2] - z);
                    if d < max_d && best.map_or(true, |b| d < b.0) {
                        best = Some((d, p));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }
}

pub struct RoadNet {
    pub roads: Vec<Road>,
    pub grid: PathGrid,
    pub failed_links: usize,
}

fn trace(pg: &mut PathGrid, astar: &mut Astar, a: usize, b: usize, class: RoadClass) -> Option<Vec<usize>> {
    for relax in [1.0, 1.3, 1.8] {
        if let Some(p) = astar.find(pg, a, b, class, relax) {
            return Some(p);
        }
    }
    None
}

/// 把路徑切成「新路段」，已存在的道路格子不重複生成
fn add_path(
    net_roads: &mut Vec<Road>,
    index: &mut PointIndex,
    pg: &mut PathGrid,
    path: &[usize],
    class: RoadClass,
    h: &Grid<f32>,
    hy: &Hydro,
) {
    let n = pg.n;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < path.len() {
        if pg.road[path[i]] == 0 {
            let s = i.saturating_sub(1);
            let mut e = i;
            while e < path.len() && pg.road[path[e]] == 0 {
                e += 1;
            }
            let e = e.min(path.len() - 1);
            runs.push((s, e));
            i = e + 1;
        } else {
            i += 1;
        }
    }
    for &(s, e) in &runs {
        if e <= s {
            continue;
        }
        let raw: Vec<[f32; 2]> = path[s..=e]
            .iter()
            .map(|&k| [((k % n) as f32 + 0.5) * pg.cell, ((k / n) as f32 + 0.5) * pg.cell])
            .collect();
        let mut xz = chaikin(&dp_simplify(&raw, pg.cell * 0.7), 3);
        let mut pin_s = None;
        let mut pin_e = None;
        if pg.road[path[s]] != 0 {
            if let Some(p) = index.nearest(xz[0][0], xz[0][1], net_roads, pg.cell * 3.0) {
                xz[0] = [p[0], p[2]];
                pin_s = Some(p[1]);
            }
        }
        if pg.road[path[e]] != 0 {
            let l = xz.len() - 1;
            if let Some(p) = index.nearest(xz[l][0], xz[l][1], net_roads, pg.cell * 3.0) {
                xz[l] = [p[0], p[2]];
                pin_e = Some(p[1]);
            }
        }
        let xz = resample(&xz, 6.0);
        if xz.len() < 2 {
            continue;
        }
        let road = build_profile(&xz, class, h, hy, pin_s, pin_e);
        net_roads.push(road);
        let id = (net_roads.len() - 1) as u32;
        index.add(id, net_roads);
    }
    for (i, &k) in path.iter().enumerate() {
        if pg.road[k] == 0 {
            pg.road[k] = class as u8 + 1;
        }
        // 馬步移動時補上中間格，保證道路格 8 連通
        if i > 0 {
            let p = path[i - 1];
            let (x0, y0) = ((p % n) as i32, (p / n) as i32);
            let (x1, y1) = ((k % n) as i32, (k / n) as i32);
            if (x1 - x0).abs() + (y1 - y0).abs() == 3 {
                let m = ((y0 + y1) / 2) as usize * n + ((x0 + x1) / 2) as usize;
                let m2 = ((y0 + y1 + 1) / 2) as usize * n + ((x0 + x1 + 1) / 2) as usize;
                for j in [m, m2] {
                    if pg.road[j] == 0 && pg.water[j] != Water::Ocean as u8 {
                        pg.road[j] = class as u8 + 1;
                    }
                }
            }
        }
    }
}

pub fn build_network(h: &Grid<f32>, hy: &Hydro, urban: &Grid<u8>, settlements: &[Settlement], size: f32) -> RoadNet {
    let mut pg = PathGrid::new(h, hy, urban);
    let mut astar = Astar::new(pg.n);
    let mut roads: Vec<Road> = Vec::new();
    let mut index = PointIndex::new(size);
    let sf = size / 12288.0;
    let ns = settlements.len();

    // 最小生成樹（Kruskal）
    let mut edges: Vec<(f32, usize, usize)> = Vec::new();
    for i in 0..ns {
        for j in i + 1..ns {
            let d = libm::hypotf(settlements[i].x - settlements[j].x, settlements[i].z - settlements[j].z);
            edges.push((d, i, j));
        }
    }
    edges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut parent: Vec<usize> = (0..ns).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let nx = p[c];
            p[c] = r;
            c = nx;
        }
        r
    }
    let mut links: Vec<(usize, usize)> = Vec::new();
    let mut gd = vec![vec![f32::INFINITY; ns]; ns];
    for i in 0..ns {
        gd[i][i] = 0.0;
    }
    for &(d, i, j) in &edges {
        let (a, b) = (find(&mut parent, i), find(&mut parent, j));
        if a != b {
            parent[a] = b;
            links.push((i, j));
            gd[i][j] = d;
            gd[j][i] = d;
        }
    }
    // 補充邊：繞路太遠的近鄰對
    let floyd = |gd: &mut Vec<Vec<f32>>| {
        for k in 0..ns {
            for i in 0..ns {
                for j in 0..ns {
                    let v = gd[i][k] + gd[k][j];
                    if v < gd[i][j] {
                        gd[i][j] = v;
                    }
                }
            }
        }
    };
    floyd(&mut gd);
    for &(d, i, j) in &edges {
        if d < 8000.0 * sf && gd[i][j] > 1.35 * d && !links.contains(&(i, j)) {
            links.push((i, j));
            gd[i][j] = d;
            gd[j][i] = d;
            floyd(&mut gd);
        }
    }
    let class_of = |i: usize, j: usize| {
        let big = |t: Tier| t != Tier::Town;
        if big(settlements[i].tier) && big(settlements[j].tier) {
            RoadClass::Highway
        } else {
            RoadClass::Regional
        }
    };
    links.sort_by(|a, b| {
        let ca = class_of(a.0, a.1) as u8;
        let cb = class_of(b.0, b.1) as u8;
        ca.cmp(&cb).then(a.cmp(b))
    });
    let mut failed = 0;
    for &(i, j) in &links {
        let class = class_of(i, j);
        let (ax, ay) = pg.cell_of(settlements[i].x, settlements[i].z);
        let (bx, by) = pg.cell_of(settlements[j].x, settlements[j].z);
        let a = ay * pg.n + ax;
        let b = by * pg.n + bx;
        match trace(&mut pg, &mut astar, a, b, class) {
            Some(path) => add_path(&mut roads, &mut index, &mut pg, &path, class, h, hy),
            None => failed += 1,
        }
    }

    // 環島海岸公路：沿射線找海岸附近的路點，依序相連
    let n = pg.n;
    let half = size * 0.5;
    let k_ring = (14.0 * sf).round().max(8.0) as usize;
    let mut ring: Vec<usize> = Vec::new();
    for i in 0..k_ring {
        let a = i as f32 / k_ring as f32 * core::f32::consts::TAU;
        let (dx, dz) = (libm::cosf(a), libm::sinf(a));
        let mut last_land = None;
        let mut t = 0.0;
        while t < half * 1.45 {
            let (x, z) = (half + dx * t, half + dz * t);
            if x < 0.0 || z < 0.0 || x >= size || z >= size {
                break;
            }
            let (cx, cz) = pg.cell_of(x, z);
            let k = cz * n + cx;
            if pg.water[k] == Water::Ocean as u8 {
                break;
            }
            if pg.water[k] == 0 {
                last_land = Some(t);
            }
            t += pg.cell * 0.5;
        }
        if let Some(t) = last_land {
            // 往內陸退一點，避開懸崖邊
            let t = (t - 140.0).max(0.0);
            let (cx, cz) = pg.cell_of(half + dx * t, half + dz * t);
            let k = cz * n + cx;
            if pg.water[k] == 0 {
                ring.push(k);
            }
        }
    }
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        if a == b {
            continue;
        }
        if let Some(path) = trace(&mut pg, &mut astar, a, b, RoadClass::Regional) {
            add_path(&mut roads, &mut index, &mut pg, &path, RoadClass::Regional, h, hy);
        }
    }

    // 山道：連到幾座高峰
    let mut peaks: Vec<(f32, usize)> = Vec::new();
    let r = 12i32;
    for y in (r as usize..n - r as usize).step_by(3) {
        for x in (r as usize..n - r as usize).step_by(3) {
            let k = y * n + x;
            let hk = pg.h[k];
            if hk < 220.0 || pg.water[k] != 0 {
                continue;
            }
            let mut is_max = true;
            'o: for dy in (-r..=r).step_by(2) {
                for dx in (-r..=r).step_by(2) {
                    let j = (y as i32 + dy) as usize * n + (x as i32 + dx) as usize;
                    if pg.h[j] > hk {
                        is_max = false;
                        break 'o;
                    }
                }
            }
            if is_max {
                peaks.push((hk, k));
            }
        }
    }
    peaks.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let want = (3.0 + 2.0 * sf * sf).round() as usize;
    let mut chosen: Vec<usize> = Vec::new();
    for &(_, k) in &peaks {
        if chosen.len() >= want {
            break;
        }
        let (x, y) = ((k % n) as f32, (k / n) as f32);
        if chosen.iter().all(|&c| libm::hypotf((c % n) as f32 - x, (c / n) as f32 - y) * pg.cell > 2500.0 * sf) {
            chosen.push(k);
        }
    }
    for &peak in &chosen {
        // 山頂附近找一處較平的停車點
        let (px, py) = ((peak % n) as i32, (peak / n) as i32);
        let mut target = peak;
        let mut best = f32::MAX;
        for dy in -4..=4 {
            for dx in -4..=4 {
                let (x, y) = (px + dx, py + dy);
                if x <= 0 || y <= 0 || x >= n as i32 - 1 || y >= n as i32 - 1 {
                    continue;
                }
                let k = y as usize * n + x as usize;
                let s = (pg.h[k + 1] - pg.h[k - 1]).abs() + (pg.h[k + n] - pg.h[k - n]).abs();
                if s < best {
                    best = s;
                    target = k;
                }
            }
        }
        // 先連到最近的現有道路，再往反方向連第二條，形成翻山的穿越路線
        let (tx, ty) = ((target % n) as f32, (target / n) as f32);
        let mut first_dir: Option<(f32, f32)> = None;
        for leg in 0..2 {
            let mut near = None;
            let mut nd = f32::MAX;
            for k in 0..n * n {
                if pg.road[k] == 0 {
                    continue;
                }
                let (dx, dy) = ((k % n) as f32 - tx, (k / n) as f32 - ty);
                let d = libm::hypotf(dx, dy);
                if d < 6.0 {
                    continue;
                }
                if let Some((fx, fy)) = first_dir {
                    if (dx * fx + dy * fy) / d > -0.2 {
                        continue;
                    }
                }
                if d < nd {
                    nd = d;
                    near = Some((k, dx / d, dy / d));
                }
            }
            let Some((start, ux, uy)) = near else { break };
            if leg == 1 && nd * pg.cell > 4500.0 * sf {
                break;
            }
            match trace(&mut pg, &mut astar, start, target, RoadClass::Mountain) {
                Some(path) => add_path(&mut roads, &mut index, &mut pg, &path, RoadClass::Mountain, h, hy),
                None => {}
            }
            first_dir = Some((ux, uy));
        }
    }

    RoadNet { roads, grid: pg, failed_links: failed }
}
