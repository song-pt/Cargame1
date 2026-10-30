//! 2D 俯視地圖（RGBA），用於除錯圖片與遊戲內小地圖。

use crate::city::BuildingKind;
use crate::hydro::Water;
use crate::landcover::Cover;
use crate::roads::RoadClass;
use crate::settle::Style;
use crate::World;

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Image {
    fn put(&mut self, x: i32, y: i32, c: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        for k in 0..3 {
            let o = self.rgba[i + k] as f32;
            self.rgba[i + k] = (o + (c[k] as f32 - o) * a) as u8;
        }
    }
    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 3], a: f32) {
        let ri = r.ceil() as i32 + 1;
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                let d = libm::hypotf(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let cov = (r + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(x, y, c, a * cov);
                }
            }
        }
    }
    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: [u8; 3], a: f32) {
        let len = libm::hypotf(x1 - x0, y1 - y0);
        let steps = (len / 0.5).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            self.disc(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, r, c, a);
        }
    }
}

pub fn render(world: &World, px: usize) -> Image {
    let n = world.height.n;
    let mut img = Image { w: px, h: px, rgba: vec![255; px * px * 4] };
    let scale = px as f32 / world.size;
    for y in 0..px {
        for x in 0..px {
            let gx = (x * n / px).min(n - 1);
            let gy = (y * n / px).min(n - 1);
            let k = gy * n + gx;
            let e = world.height.data[k];
            let wk = world.hydro.kind.data[k];
            // 山體陰影（西北光）
            let sh = {
                let (a, b) = (world.height.get_clamped(gx as i32 - 1, gy as i32 - 1), world.height.get_clamped(gx as i32 + 1, gy as i32 + 1));
                (((a - b) / (world.height.cell * 2.0)) * 1.6).clamp(-0.6, 0.6)
            };
            let c: [f32; 3] = if wk == Water::Ocean as u8 {
                let d = (-e / 45.0).clamp(0.0, 1.0);
                [40.0 - 20.0 * d, 110.0 - 50.0 * d, 160.0 - 40.0 * d]
            } else if wk != 0 {
                [55.0, 125.0, 175.0]
            } else {
                let base = match world.cover.data[k] {
                    x if x == Cover::Forest as u8 => [58.0, 96.0, 52.0],
                    x if x == Cover::Rock as u8 => [138.0, 128.0, 116.0],
                    x if x == Cover::Beach as u8 => [222.0, 206.0, 160.0],
                    x if x == Cover::Tea as u8 => [96.0, 150.0, 70.0],
                    x if x == Cover::Flowers as u8 => [176.0, 160.0, 120.0],
                    x if x == Cover::Urban as u8 => {
                        let u = world.urban.data[k];
                        match world.settlements[(u - 1) as usize].style {
                            Style::Old => [196.0, 170.0, 150.0],
                            Style::New => [176.0, 182.0, 196.0],
                        }
                    }
                    _ => [120.0, 160.0, 90.0],
                };
                let tint = (e / 700.0).clamp(0.0, 0.4);
                [base[0] * (1.0 - tint) + 200.0 * tint, base[1] * (1.0 - tint) + 190.0 * tint, base[2] * (1.0 - tint) + 180.0 * tint]
            };
            let s = 1.0 + sh;
            let i = (y * px + x) * 4;
            img.rgba[i] = (c[0] * s).clamp(0.0, 255.0) as u8;
            img.rgba[i + 1] = (c[1] * s).clamp(0.0, 255.0) as u8;
            img.rgba[i + 2] = (c[2] * s).clamp(0.0, 255.0) as u8;
        }
    }
    for b in &world.buildings {
        if matches!(b.kind, BuildingKind::Tower | BuildingKind::Landmark) {
            let r = if b.kind == BuildingKind::Landmark { 2.5 } else { 1.2 };
            img.disc(b.x * scale, b.z * scale, r, [60, 60, 80], 0.9);
        }
    }
    let draw_roads = |roads: &[crate::roads::Road], img: &mut Image| {
        for r in roads {
            let (rad, col, a): (f32, [u8; 3], f32) = match r.class {
                RoadClass::Highway => (1.6, [250, 205, 70], 1.0),
                RoadClass::Regional => (1.1, [250, 250, 245], 1.0),
                RoadClass::Mountain => (0.9, [240, 140, 60], 1.0),
                RoadClass::Arterial => (0.9, [235, 235, 235], 0.9),
                RoadClass::Street => (0.45, [255, 255, 255], 0.55),
                RoadClass::Alley => (0.3, [255, 255, 255], 0.35),
            };
            for (w, br) in r.pts.windows(2).zip(r.bridge.windows(2)) {
                let c = if br[0] && br[1] { [120, 60, 160] } else { col };
                img.line(w[0][0] * scale, w[0][2] * scale, w[1][0] * scale, w[1][2] * scale, rad, c, a);
            }
        }
    };
    draw_roads(&world.streets, &mut img);
    draw_roads(&world.roads, &mut img);
    for p in &world.pois {
        let c = match p.kind {
            crate::places::PoiKind::Viewpoint => [255, 255, 0],
            crate::places::PoiKind::Lighthouse => [255, 255, 255],
            crate::places::PoiKind::Shrine => [230, 40, 40],
        };
        img.disc(p.x * scale, p.z * scale, 2.6, [0, 0, 0], 0.8);
        img.disc(p.x * scale, p.z * scale, 1.8, c, 1.0);
    }
    for s in &world.stations {
        img.disc(s.x * scale, s.z * scale, 3.2, [0, 0, 0], 0.9);
        img.disc(s.x * scale, s.z * scale, 2.3, [0, 200, 120], 1.0);
    }
    for s in &world.settlements {
        img.disc(s.x * scale, s.z * scale, 3.0, [20, 20, 20], 1.0);
        if let Some(c) = &s.cbd {
            img.disc(c.x * scale, c.z * scale, 4.0, [30, 60, 220], 1.0);
        }
    }
    img
}
