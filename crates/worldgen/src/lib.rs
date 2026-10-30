//! 程序化世界生成。由「種子 + 大小」確定性地生成整個世界。
//!
//! 分層：宏觀地形 → 侵蝕 → 水系 → 聚落 → 城際路網 → 城市街道與建築 → 加油站與景點 → 地表類型。
//! 每一層只依賴上一層的輸出。

pub mod city;
pub mod fine;
pub mod grid;
pub mod hydro;
pub mod landcover;
pub mod map;
pub mod noise;
pub mod places;
pub mod rng;
pub mod roads;
pub mod settle;
pub mod terrain;
pub mod validate;

use city::Building;
use grid::Grid;
use hydro::Hydro;
use places::{Poi, Station};
use roads::{PathGrid, Road};
use settle::Settlement;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldSize {
    Small,
    Medium,
    Large,
}

impl WorldSize {
    /// 世界邊長（米）。島嶼形狀，陸地約佔 2/3。
    pub fn meters(self) -> f32 {
        match self {
            WorldSize::Small => 8192.0,
            WorldSize::Medium => 12288.0,
            WorldSize::Large => 16384.0,
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "small" => WorldSize::Small,
            "large" => WorldSize::Large,
            _ => WorldSize::Medium,
        }
    }
}

/// 官方世界的種子
pub const OFFICIAL_SEED: u64 = 20261030;

pub const MACRO_N: usize = 1024;

pub struct World {
    pub seed: u64,
    pub size: f32,
    pub height: Grid<f32>,
    pub hydro: Hydro,
    pub settlements: Vec<Settlement>,
    pub urban: Grid<u8>,
    pub land_area_m2: f32,
    pub roads: Vec<Road>,
    pub streets: Vec<Road>,
    pub path_grid: PathGrid,
    pub failed_links: usize,
    pub buildings: Vec<Building>,
    pub stations: Vec<Station>,
    pub pois: Vec<Poi>,
    pub cover: Grid<u8>,
    pub timings: Vec<(&'static str, f64)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Terrain,
    Erosion,
    Water,
    Settlements,
    Roads,
    Cities,
    Places,
    Landcover,
    Done,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Terrain => "terrain",
            Stage::Erosion => "erosion",
            Stage::Water => "water",
            Stage::Settlements => "settlements",
            Stage::Roads => "roads",
            Stage::Cities => "cities",
            Stage::Places => "places",
            Stage::Landcover => "landcover",
            Stage::Done => "done",
        }
    }
    pub fn progress(self) -> f32 {
        match self {
            Stage::Terrain => 0.0,
            Stage::Erosion => 0.08,
            Stage::Water => 0.40,
            Stage::Settlements => 0.50,
            Stage::Roads => 0.58,
            Stage::Cities => 0.80,
            Stage::Places => 0.90,
            Stage::Landcover => 0.95,
            Stage::Done => 1.0,
        }
    }
}

/// 分階段生成，讓網頁版可以在每一步之間更新進度條。
pub struct Builder {
    pub seed: u64,
    pub size: f32,
    pub stage: Stage,
    height: Option<Grid<f32>>,
    hydro: Option<Hydro>,
    settle: Option<settle::SettleOut>,
    net: Option<roads::RoadNet>,
    city: Option<city::CityOut>,
    places: Option<(Vec<Station>, Vec<Poi>, Vec<Building>)>,
    cover: Option<Grid<u8>>,
    pub timings: Vec<(&'static str, f64)>,
}

impl Builder {
    pub fn new(seed: u64, size: WorldSize) -> Self {
        Self {
            seed,
            size: size.meters(),
            stage: Stage::Terrain,
            height: None,
            hydro: None,
            settle: None,
            net: None,
            city: None,
            places: None,
            cover: None,
            timings: Vec::new(),
        }
    }

    /// 執行下一個階段；`now` 為取得毫秒時間的函數（各平台不同）
    pub fn step(&mut self, now: &dyn Fn() -> f64) -> Stage {
        let t0 = now();
        let seed32 = (self.seed ^ (self.seed >> 32)) as u32;
        let stage = self.stage;
        match stage {
            Stage::Terrain => {
                self.height = Some(terrain::generate_base(MACRO_N, self.size, seed32));
                self.stage = Stage::Erosion;
            }
            Stage::Erosion => {
                let h = self.height.as_mut().unwrap();
                terrain::erode(h, self.seed, MACRO_N * MACRO_N / 4);
                self.stage = Stage::Water;
            }
            Stage::Water => {
                let h = self.height.as_mut().unwrap();
                let sf = self.size / 12288.0;
                self.hydro = Some(hydro::build(h, 1.6e6, 2.5e6 * sf * sf));
                self.stage = Stage::Settlements;
            }
            Stage::Settlements => {
                let h = self.height.as_mut().unwrap();
                self.settle = Some(settle::place(h, self.hydro.as_ref().unwrap(), self.size, self.seed));
                self.stage = Stage::Roads;
            }
            Stage::Roads => {
                let s = self.settle.as_ref().unwrap();
                self.net = Some(roads::build_network(
                    self.height.as_ref().unwrap(),
                    self.hydro.as_ref().unwrap(),
                    &s.urban,
                    &s.list,
                    self.size,
                ));
                self.stage = Stage::Cities;
            }
            Stage::Cities => {
                let s = self.settle.as_ref().unwrap();
                self.city = Some(city::generate(
                    self.height.as_ref().unwrap(),
                    self.hydro.as_ref().unwrap(),
                    &s.urban,
                    &self.net.as_ref().unwrap().grid,
                    &s.list,
                    self.seed,
                ));
                self.stage = Stage::Places;
            }
            Stage::Places => {
                let s = self.settle.as_ref().unwrap();
                let net = self.net.as_ref().unwrap();
                let h = self.height.as_ref().unwrap();
                let stations = places::place_stations(&net.roads, &net.grid, &s.list, h, self.hydro.as_ref().unwrap());
                let pois = places::place_pois(&net.roads, h, self.hydro.as_ref().unwrap(), &s.urban, &net.grid, self.seed);
                let extra = places::poi_buildings(&pois, &stations, self.seed);
                self.places = Some((stations, pois, extra));
                self.stage = Stage::Landcover;
            }
            Stage::Landcover => {
                let s = self.settle.as_ref().unwrap();
                self.cover = Some(landcover::classify(self.height.as_ref().unwrap(), self.hydro.as_ref().unwrap(), &s.urban, seed32));
                self.stage = Stage::Done;
            }
            Stage::Done => {}
        }
        self.timings.push((stage.name(), now() - t0));
        self.stage
    }

    pub fn finish(mut self, now: &dyn Fn() -> f64) -> World {
        while self.stage != Stage::Done {
            self.step(now);
        }
        let s = self.settle.unwrap();
        let net = self.net.unwrap();
        let mut city = self.city.unwrap();
        let (stations, pois, extra) = self.places.unwrap();
        city.buildings.extend(extra);
        World {
            seed: self.seed,
            size: self.size,
            height: self.height.unwrap(),
            hydro: self.hydro.unwrap(),
            settlements: s.list,
            urban: s.urban,
            land_area_m2: s.land_area_m2,
            roads: net.roads,
            streets: city.streets,
            path_grid: net.grid,
            failed_links: net.failed_links,
            buildings: city.buildings,
            stations,
            pois,
            cover: self.cover.unwrap(),
            timings: self.timings,
        }
    }
}

pub fn generate(seed: u64, size: WorldSize, now: &dyn Fn() -> f64) -> World {
    Builder::new(seed, size).finish(now)
}

impl World {
    /// 世界內容的雜湊，用於確定性測試
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        let mut mix = |v: u32| {
            h ^= v as u64;
            h = h.wrapping_mul(0x0100_0000_01B3);
        };
        for v in &self.height.data {
            mix(v.to_bits());
        }
        for r in self.roads.iter().chain(&self.streets) {
            for p in &r.pts {
                mix(p[0].to_bits());
                mix(p[1].to_bits());
                mix(p[2].to_bits());
            }
        }
        for b in &self.buildings {
            mix(b.x.to_bits());
            mix(b.sy.to_bits());
        }
        for s in &self.stations {
            mix(s.x.to_bits());
        }
        h
    }

    /// 找一個適合出生的位置：城際道路上、最大城市附近
    pub fn spawn_point(&self) -> ([f32; 3], f32) {
        let target = self
            .settlements
            .iter()
            .find(|s| s.style == settle::Style::Old && s.tier == settle::Tier::Major)
            .or(self.settlements.first());
        let (tx, tz) = target.map(|s| (s.x + s.radius * 1.1, s.z)).unwrap_or((self.size * 0.5, self.size * 0.5));
        let mut best: Option<(f32, [f32; 3], f32)> = None;
        for r in &self.roads {
            for i in 1..r.pts.len() {
                if r.bridge[i] {
                    continue;
                }
                let p = r.pts[i];
                let d = libm::hypotf(p[0] - tx, p[2] - tz);
                if best.as_ref().map_or(true, |b| d < b.0) {
                    let q = r.pts[i - 1];
                    best = Some((d, p, libm::atan2f(p[0] - q[0], p[2] - q[2])));
                }
            }
        }
        best.map(|b| (b.1, b.2)).unwrap_or(([self.size * 0.5, 50.0, self.size * 0.5], 0.0))
    }
}
