//! 地表類型：森林、岩石、沙灘、茶園、花田等。

use crate::grid::Grid;
use crate::hydro::{Hydro, Water};
use crate::noise::fbm;
use crate::settle::distance_field;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cover {
    Grass = 0,
    Forest = 1,
    Rock = 2,
    Beach = 3,
    Tea = 4,
    Flowers = 5,
    Urban = 6,
    Water = 7,
}

pub fn classify(h: &Grid<f32>, hy: &Hydro, urban: &Grid<u8>, seed: u32) -> Grid<u8> {
    let n = h.n;
    let mut c = Grid::new(n, h.cell, Cover::Grass as u8);
    let d_ocean = distance_field(n, |k| hy.kind.data[k] == Water::Ocean as u8);
    for y in 0..n {
        for x in 0..n {
            let k = y * n + x;
            let (wx, wz) = h.center(x, y);
            let e = h.data[k];
            let s = h.slope_at(x, y);
            let v = if hy.kind.data[k] != Water::None as u8 {
                Cover::Water
            } else if urban.data[k] != 0 {
                Cover::Urban
            } else if e < 4.0 && d_ocean[k] <= 5 && s < 0.12 {
                Cover::Beach
            } else if s > 0.75 || e > 640.0 + 80.0 * fbm(wx / 500.0, wz / 500.0, seed ^ 0x71, 2) {
                Cover::Rock
            } else {
                let forest = fbm(wx / 900.0, wz / 900.0, seed ^ 0x72, 4) + (e / 500.0).min(0.5) - 0.1;
                let patch = fbm(wx / 450.0, wz / 450.0, seed ^ 0x73, 3);
                if e > 35.0 && e < 320.0 && s > 0.06 && s < 0.3 && patch > 0.32 {
                    Cover::Tea
                } else if forest > 0.12 && d_ocean[k] > 8 {
                    Cover::Forest
                } else if s < 0.1 && patch < -0.35 {
                    Cover::Flowers
                } else {
                    Cover::Grass
                }
            };
            c.data[k] = v as u8;
        }
    }
    c
}
