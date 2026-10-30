//! 規則網格。格點 i 位於世界座標 (i + 0.5) * cell。

#[derive(Clone, Debug)]
pub struct Grid<T> {
    pub n: usize,
    pub cell: f32,
    pub data: Vec<T>,
}

impl<T: Copy> Grid<T> {
    pub fn new(n: usize, cell: f32, v: T) -> Self {
        Self { n, cell, data: vec![v; n * n] }
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.n + x
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> T {
        self.data[y * self.n + x]
    }

    #[inline]
    pub fn get_clamped(&self, x: i32, y: i32) -> T {
        let m = self.n as i32 - 1;
        self.data[(y.clamp(0, m) as usize) * self.n + x.clamp(0, m) as usize]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: T) {
        let i = y * self.n + x;
        self.data[i] = v;
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.n && (y as usize) < self.n
    }

    /// 世界座標 → 最近格點
    #[inline]
    pub fn cell_of(&self, wx: f32, wz: f32) -> (i32, i32) {
        (libm::floorf(wx / self.cell) as i32, libm::floorf(wz / self.cell) as i32)
    }

    #[inline]
    pub fn center(&self, x: usize, y: usize) -> (f32, f32) {
        ((x as f32 + 0.5) * self.cell, (y as f32 + 0.5) * self.cell)
    }
}

impl Grid<f32> {
    /// 雙線性取樣（世界座標）
    pub fn sample(&self, wx: f32, wz: f32) -> f32 {
        let fx = wx / self.cell - 0.5;
        let fy = wz / self.cell - 0.5;
        let x0 = libm::floorf(fx);
        let y0 = libm::floorf(fy);
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as i32, y0 as i32);
        let a = self.get_clamped(x0, y0);
        let b = self.get_clamped(x0 + 1, y0);
        let c = self.get_clamped(x0, y0 + 1);
        let d = self.get_clamped(x0 + 1, y0 + 1);
        let ab = a + (b - a) * tx;
        let cd = c + (d - c) * tx;
        ab + (cd - ab) * ty
    }

    /// 坡度（高差 / 水平距離）
    pub fn slope_at(&self, x: usize, y: usize) -> f32 {
        let (x, y) = (x as i32, y as i32);
        let dx = self.get_clamped(x + 1, y) - self.get_clamped(x - 1, y);
        let dy = self.get_clamped(x, y + 1) - self.get_clamped(x, y - 1);
        libm::sqrtf(dx * dx + dy * dy) / (2.0 * self.cell)
    }

    /// 簡單的可分離方框模糊，半徑 r 格
    pub fn blurred(&self, r: usize) -> Grid<f32> {
        let n = self.n;
        let mut tmp = vec![0.0f32; n * n];
        let mut out = self.clone();
        let r = r as i32;
        let inv = 1.0 / (2 * r + 1) as f32;
        for y in 0..n {
            let mut acc = 0.0;
            for k in -r..=r {
                acc += self.get_clamped(k, y as i32);
            }
            for x in 0..n {
                tmp[y * n + x] = acc * inv;
                acc += self.get_clamped(x as i32 + r + 1, y as i32) - self.get_clamped(x as i32 - r, y as i32);
            }
        }
        let t = Grid { n, cell: self.cell, data: tmp };
        for x in 0..n {
            let mut acc = 0.0;
            for k in -r..=r {
                acc += t.get_clamped(x as i32, k);
            }
            for y in 0..n {
                out.data[y * n + x] = acc * inv;
                acc += t.get_clamped(x as i32, y as i32 + r + 1) - t.get_clamped(x as i32, y as i32 - r);
            }
        }
        out
    }
}
