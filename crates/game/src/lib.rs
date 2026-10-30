//! 網頁版入口：世界分階段生成、遊戲迴圈、輸入、HUD、除錯資訊。

pub mod lod;
pub mod meshes;
pub mod render;
pub mod shaders;
pub mod sky;

use glam::{Mat4, Quat, Vec3};
use render::{FrameParams, PartInstance, Renderer};
use std::collections::HashSet;
use vehicle::{Car, CarParams, Ground, Input};
use wasm_bindgen::prelude::*;
use worldgen::fine::FineData;
use worldgen::{Builder, Stage, World, WorldSize};

fn now_ms() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.0)
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// 分階段生成世界，讓網頁可以在每一步之間更新進度條
#[wasm_bindgen]
pub struct WorldLoader {
    builder: Option<Builder>,
    world: Option<World>,
    fine: Option<FineData>,
    fine_ms: f64,
}

#[wasm_bindgen]
impl WorldLoader {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: &str, size: &str) -> WorldLoader {
        let seed = seed.trim().parse::<u64>().unwrap_or_else(|_| {
            // 非數字種子：取字串雜湊
            seed.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01B3))
        });
        WorldLoader { builder: Some(Builder::new(seed, WorldSize::from_str(size))), world: None, fine: None, fine_ms: 0.0 }
    }

    /// 執行一步，回傳進度 0..1；1 表示完成
    pub fn step(&mut self) -> f32 {
        if let Some(b) = self.builder.as_mut() {
            let st = b.step(&now_ms);
            if st == Stage::Done {
                let b = self.builder.take().unwrap();
                self.world = Some(b.finish(&now_ms));
            }
            return st.progress() * 0.92;
        }
        if self.fine.is_none() {
            let t0 = now_ms();
            self.fine = Some(worldgen::fine::build(self.world.as_ref().unwrap(), 2048));
            self.fine_ms = now_ms() - t0;
        }
        1.0
    }

    pub fn stage(&self) -> String {
        match &self.builder {
            Some(b) => b.stage.name().to_string(),
            None => if self.fine.is_none() { "detail".into() } else { "done".into() },
        }
    }
}

struct GroundRef<'a>(&'a FineData);
impl Ground for GroundRef<'_> {
    fn height(&self, x: f32, z: f32) -> f32 {
        self.0.height_at(x, z)
    }
    fn grip(&self, x: f32, z: f32) -> f32 {
        0.72 + 0.28 * self.0.road_at(x, z)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum CamMode {
    Chase,
    Hood,
}

#[wasm_bindgen]
pub struct Game {
    r: Renderer,
    world: World,
    fine: FineData,
    car: Car,
    keys: HashSet<String>,
    tilt: Option<f32>,
    steer: f32,
    acc: f64,
    last: f64,
    time_s: f64,
    hours: f32,
    time_speed: f32,
    cam_yaw: f32,
    cam_pos: Vec3,
    orbit: (f32, f32),
    dragging: bool,
    cam_mode: CamMode,
    shift_up: bool,
    shift_down: bool,
    fps_hist: Vec<f32>,
    cpu_ms: f32,
    refueling: bool,
    seed_str: String,
    gen_timings: String,
    quality_high: bool,
    map: Vec<u8>,
    top_view: f32,
}

#[wasm_bindgen]
pub async fn create_game(canvas: web_sys::HtmlCanvasElement, loader: WorldLoader, quality: String) -> Result<Game, JsValue> {
    let mut loader = loader;
    while loader.step() < 1.0 {}
    let world = loader.world.take().unwrap();
    let fine = loader.fine.take().unwrap();
    let high = quality != "low";
    let r = Renderer::new(canvas, &world, &fine, high).await.map_err(|e| JsValue::from_str(&e))?;
    let (sp, heading) = world.spawn_point();
    let mut car = Car::new(CarParams::sports_coupe(), Vec3::new(sp[0], sp[1] + 1.2, sp[2]), heading);
    car.automatic = true;
    let mut timings: Vec<String> = world.timings.iter().map(|(n, ms)| format!("{}={:.0}ms", n, ms)).collect();
    timings.push(format!("detail={:.0}ms", loader.fine_ms));
    let map = worldgen::map::render(&world, 512).rgba;
    let seed_str = world.seed.to_string();
    Ok(Game {
        r,
        cam_pos: car.pos + Vec3::new(0.0, 3.0, -8.0),
        cam_yaw: heading,
        world,
        fine,
        car,
        keys: HashSet::new(),
        tilt: None,
        steer: 0.0,
        acc: 0.0,
        last: now_ms(),
        time_s: 0.0,
        hours: 16.5,
        time_speed: 0.0,
        orbit: (0.0, 0.0),
        dragging: false,
        cam_mode: CamMode::Chase,
        shift_up: false,
        shift_down: false,
        fps_hist: Vec::new(),
        cpu_ms: 0.0,
        refueling: false,
        seed_str,
        gen_timings: timings.join(" "),
        quality_high: high,
        map,
        top_view: 0.0,
    })
}

fn wrap_angle(a: f32) -> f32 {
    let mut a = a % core::f32::consts::TAU;
    if a > core::f32::consts::PI {
        a -= core::f32::consts::TAU;
    }
    if a < -core::f32::consts::PI {
        a += core::f32::consts::TAU;
    }
    a
}

#[wasm_bindgen]
impl Game {
    pub fn key(&mut self, code: &str, down: bool) {
        if down && !self.keys.contains(code) {
            match code {
                "KeyE" | "ShiftLeft" | "ShiftRight" | "TouchUp" => self.shift_up = true,
                "KeyQ" | "ControlLeft" | "ControlRight" | "TouchDown" => self.shift_down = true,
                "KeyR" => self.reset_car(),
                "KeyC" => self.cam_mode = if self.cam_mode == CamMode::Chase { CamMode::Hood } else { CamMode::Chase },
                "KeyT" => self.car.automatic = !self.car.automatic,
                _ => {}
            }
        }
        if down {
            self.keys.insert(code.to_string());
        } else {
            self.keys.remove(code);
        }
    }

    pub fn clear_keys(&mut self) {
        self.keys.clear();
    }

    /// 陀螺儀轉向：-1..1；傳入 NaN 表示關閉
    pub fn set_tilt(&mut self, v: f32) {
        self.tilt = if v.is_nan() { None } else { Some(v.clamp(-1.0, 1.0)) };
    }

    pub fn orbit(&mut self, dx: f32, dy: f32, dragging: bool) {
        self.dragging = dragging;
        self.orbit.0 = wrap_angle(self.orbit.0 - dx * 0.006);
        self.orbit.1 = (self.orbit.1 + dy * 0.004).clamp(-0.35, 0.9);
    }

    pub fn set_hours(&mut self, h: f32) {
        self.hours = h.rem_euclid(24.0);
    }

    pub fn set_time_speed(&mut self, s: f32) {
        self.time_speed = s;
    }

    pub fn set_transmission(&mut self, automatic: bool) {
        self.car.automatic = automatic;
    }

    pub fn set_fuel_enabled(&mut self, on: bool) {
        self.car.fuel_enabled = on;
        if !on {
            self.car.fuel_l = self.car.p.fuel_capacity_l;
        }
    }

    pub fn set_assists(&mut self, tcs: bool, abs: bool) {
        self.car.tcs = tcs;
        self.car.abs = abs;
    }

    pub fn set_quality(&mut self, q: &str) {
        self.quality_high = q != "low";
        self.r.set_scale(if self.quality_high { 1.0 } else { 0.7 });
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.r.resize(w, h);
    }

    pub fn map_rgba(&self) -> Vec<u8> {
        self.map.clone()
    }

    pub fn world_size(&self) -> f32 {
        self.world.size
    }

    fn reset_car(&mut self) {
        let p = self.car.pos;
        let mut best: Option<(f32, [f32; 3], f32)> = None;
        for r in self.world.roads.iter().chain(self.world.streets.iter()) {
            for i in 1..r.pts.len() {
                let q = r.pts[i];
                let d = (q[0] - p.x).powi(2) + (q[2] - p.z).powi(2);
                if best.as_ref().map_or(true, |b| d < b.0) {
                    let a = r.pts[i - 1];
                    best = Some((d, q, (q[0] - a[0]).atan2(q[2] - a[2])));
                }
            }
        }
        if let Some((_, q, h)) = best {
            let y = self.fine.height_at(q[0], q[2]).max(q[1]);
            self.car.reset(Vec3::new(q[0], y + 1.0, q[2]), h);
            self.cam_yaw = h;
        }
    }

    fn input(&mut self, dt: f32) -> Input {
        let k = |c: &[&str]| c.iter().any(|x| self.keys.contains(*x));
        let left = k(&["ArrowLeft", "KeyA", "TouchLeft"]);
        let right = k(&["ArrowRight", "KeyD", "TouchRight"]);
        let target = if let Some(t) = self.tilt {
            t
        } else {
            (right as i32 - left as i32) as f32
        };
        // 鍵盤轉向濾波：按下時漸進，放開時較快回正
        let rate = if target == 0.0 { 5.0 } else { 3.2 };
        let d = target - self.steer;
        self.steer += d.clamp(-rate * dt, rate * dt);
        if self.tilt.is_some() {
            self.steer = target;
        }
        let inp = Input {
            throttle: if k(&["ArrowUp", "KeyW", "TouchGas"]) { 1.0 } else { 0.0 },
            brake: if k(&["ArrowDown", "KeyS", "TouchBrake"]) { 1.0 } else { 0.0 },
            // 物理模組中正轉角朝 +X（從車後看是左邊）
            steer: -self.steer,
            handbrake: k(&["Space", "TouchHand"]),
            shift_up: self.shift_up,
            shift_down: self.shift_down,
        };
        self.shift_up = false;
        self.shift_down = false;
        inp
    }

    /// 每幀呼叫。now 為 performance.now()
    pub fn frame(&mut self, now: f64) {
        let t0 = now_ms();
        let dt = ((now - self.last) / 1000.0).clamp(0.0, 0.1);
        self.last = now;
        self.time_s += dt;
        self.hours = (self.hours + self.time_speed * dt as f32 / 3600.0).rem_euclid(24.0);
        let fps = if dt > 0.0 { 1.0 / dt as f32 } else { 0.0 };
        self.fps_hist.push(fps);
        if self.fps_hist.len() > 120 {
            self.fps_hist.remove(0);
        }

        // 物理（固定步長）
        let inp = self.input(dt as f32);
        self.acc += dt;
        let mut steps = 0;
        let mut first = true;
        while self.acc >= vehicle::DT as f64 && steps < 60 {
            let mut i = inp;
            if !first {
                i.shift_up = false;
                i.shift_down = false;
            }
            first = false;
            self.car.step(&i, &GroundRef(&self.fine));
            self.acc -= vehicle::DT as f64;
            steps += 1;
        }
        if self.car.pos.y < -20.0 || !self.car.pos.is_finite() {
            self.reset_car();
        }
        // 加油：在加油站旁停車
        self.refueling = false;
        if self.car.fuel_enabled && self.car.speed() < 1.0 {
            let p = self.car.pos;
            if self.world.stations.iter().any(|s| (s.x - p.x).powi(2) + (s.z - p.z).powi(2) < 32.0 * 32.0) && self.car.fuel_l < self.car.p.fuel_capacity_l {
                self.car.fuel_l = (self.car.fuel_l + 6.0 * dt as f32).min(self.car.p.fuel_capacity_l);
                self.refueling = true;
            }
        }

        // 相機
        let fwd = self.car.rot * Vec3::Z;
        let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
        let car_yaw = flat.x.atan2(flat.z);
        let k = 1.0 - (-dt as f32 * 4.0).exp();
        self.cam_yaw += wrap_angle(car_yaw - self.cam_yaw) * k;
        if !self.dragging {
            let r = 1.0 - (-dt as f32 * 1.5).exp();
            self.orbit.0 -= self.orbit.0 * r;
            self.orbit.1 -= self.orbit.1 * r;
        }
        let (eye, target) = match self.cam_mode {
            CamMode::Chase => {
                let yaw = self.cam_yaw + self.orbit.0;
                let dist = 6.4;
                let height = 2.1 + self.orbit.1 * 4.0;
                let mut eye = self.car.pos + Vec3::new(-yaw.sin() * dist, height, -yaw.cos() * dist);
                let gh = self.fine.height_at(eye.x, eye.z);
                eye.y = eye.y.max(gh + 0.6);
                let target = self.car.pos + Vec3::new(0.0, 0.9, 0.0) + flat * 2.0;
                let kp = 1.0 - (-dt as f32 * 12.0).exp();
                self.cam_pos = self.cam_pos.lerp(eye, kp);
                (self.cam_pos, target)
            }
            CamMode::Hood => {
                let eye = self.car.pos + self.car.rot * Vec3::new(0.0, 0.62, 0.4);
                self.cam_pos = eye;
                (eye, eye + fwd * 10.0)
            }
        };
        let (eye, target) = if self.top_view > 0.0 {
            let e = self.car.pos + Vec3::new(0.0, self.top_view, -self.top_view * 0.6);
            (e, self.car.pos)
        } else {
            (eye, target)
        };
        let aspect = self.r.aspect();
        // 直式螢幕維持水平視角約 75°
        let fov_y = if aspect >= 1.0 { 55f32.to_radians() } else { 2.0 * ((37.5f32.to_radians()).tan() / aspect).atan() };
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, target - eye, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov_y, aspect, 0.15);
        let view_proj = proj * view;

        let light = sky::lighting(self.hours);
        let parts = self.car_parts(eye, light.night);
        let fp = FrameParams {
            view_proj,
            cam: eye,
            time: self.time_s as f32,
            light_dir: light.light_dir,
            light_color: light.light_color,
            sun_dir: light.sun_dir,
            night: light.night,
            exposure: light.exposure,
            fog_density: 0.00011,
            cloud_cover: 0.45,
            parts,
        };
        self.r.render(&fp);
        self.cpu_ms = self.cpu_ms * 0.9 + (now_ms() - t0) as f32 * 0.1;
    }

    fn car_parts(&self, cam: Vec3, _night: f32) -> Vec<PartInstance> {
        let base = Mat4::from_rotation_translation(self.car.rot, self.car.pos - cam);
        let paint = [0.55, 0.04, 0.05, 0.0];
        let mut boxes: Vec<PartInstance> = Vec::new();
        let push = |v: &mut Vec<PartInstance>, off: Vec3, size: Vec3, rot: Quat, color: [f32; 4]| {
            let m = base * Mat4::from_scale_rotation_translation(size, rot, off);
            v.push(PartInstance { m: m.to_cols_array_2d(), color });
        };
        let q0 = Quat::IDENTITY;
        let brake = if self.keys.contains("ArrowDown") || self.keys.contains("KeyS") || self.keys.contains("TouchBrake") { 4.0 } else { 1.0 };
        push(&mut boxes, Vec3::new(0.0, -0.03, 0.0), Vec3::new(1.82, 0.6, 4.35), q0, paint);
        push(&mut boxes, Vec3::new(0.0, -0.05, 1.75), Vec3::new(1.8, 0.5, 0.9), Quat::from_rotation_x(0.12), paint);
        push(&mut boxes, Vec3::new(0.0, 0.5, -0.25), Vec3::new(1.5, 0.46, 2.1), q0, [0.0, 0.0, 0.0, 1.0]);
        push(&mut boxes, Vec3::new(0.0, 0.75, -0.3), Vec3::new(1.42, 0.06, 1.6), q0, paint);
        push(&mut boxes, Vec3::new(0.0, 0.32, -2.05), Vec3::new(1.6, 0.05, 0.3), q0, [0.02, 0.02, 0.02, 0.0]);
        for x in [-0.62f32, 0.62] {
            push(&mut boxes, Vec3::new(x, 0.08, 2.17), Vec3::new(0.38, 0.12, 0.04), q0, [1.0, 1.0, 1.0, 4.0]);
            push(&mut boxes, Vec3::new(x, 0.12, -2.17), Vec3::new(0.42, 0.1, 0.04), q0, [brake, 0.0, 0.0, 5.0]);
        }
        let mut cyl: Vec<PartInstance> = Vec::new();
        for i in 0..4 {
            let w = &self.car.wheels[i];
            let mount = self.car.mount(i);
            let len = self.car.p.rest_length - w.compression;
            let c = mount - Vec3::Y * len;
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let rot = Quat::from_rotation_y(w.steer);
            let r = self.car.p.wheel_radius * 2.0;
            push(&mut cyl, c, Vec3::new(0.24, r, r), rot, [0.0, 0.0, 0.0, 2.0]);
            push(&mut cyl, c + rot * Vec3::new(side * 0.122, 0.0, 0.0), Vec3::new(0.01, r * 0.66, r * 0.66), rot, [0.0, 0.0, 0.0, 3.0]);
        }
        boxes.extend(cyl);
        boxes
    }

    /// HUD 數據（JSON）
    pub fn hud(&self) -> String {
        let gear = match self.car.gear {
            g if g > 0 => g.to_string(),
            0 => "N".into(),
            _ => "R".into(),
        };
        let p = self.car.pos;
        let yaw = {
            let f = self.car.rot * Vec3::Z;
            f.x.atan2(f.z)
        };
        format!(
            "{{\"speed\":{:.1},\"gear\":\"{}\",\"rpm\":{:.0},\"redline\":{:.0},\"fuel\":{:.3},\"fuelOn\":{},\"auto\":{},\"hours\":{:.3},\"refuel\":{},\"x\":{:.1},\"z\":{:.1},\"yaw\":{:.3},\"throttle\":{}}}",
            self.car.forward_speed().abs() * 3.6,
            gear,
            self.car.rpm,
            self.car.p.engine.redline_rpm,
            self.car.fuel_l / self.car.p.fuel_capacity_l,
            self.car.fuel_enabled,
            self.car.automatic,
            self.hours,
            self.refueling,
            p.x,
            p.z,
            yaw,
            self.keys.contains("ArrowUp") || self.keys.contains("KeyW") || self.keys.contains("TouchGas")
        )
    }

    pub fn fps(&self) -> f32 {
        if self.fps_hist.is_empty() {
            return 0.0;
        }
        self.fps_hist.iter().sum::<f32>() / self.fps_hist.len() as f32
    }

    pub fn debug_info(&self) -> String {
        let min = self.fps_hist.iter().cloned().fold(f32::MAX, f32::min);
        let c = self.car.pos;
        format!(
            "seed={} size={:.0}m quality={} render={}x{} scale={:.2}\nadapter={}\nfps avg={:.1} min={:.1} cpu={:.2}ms nodes={} buildings={} roads={}+{}\ncar=({:.1},{:.1},{:.1}) cam=({:.1},{:.1},{:.1}) hours={:.2}\ngen: {}",
            self.seed_str,
            self.world.size,
            if self.quality_high { "high" } else { "low" },
            self.r.config.width,
            self.r.config.height,
            self.r.render_scale,
            self.r.adapter_info,
            self.fps(),
            if min == f32::MAX { 0.0 } else { min },
            self.cpu_ms,
            self.r.last_nodes,
            self.world.buildings.len(),
            self.world.roads.len(),
            self.world.streets.len(),
            c.x,
            c.y,
            c.z,
            self.cam_pos.x,
            self.cam_pos.y,
            self.cam_pos.z,
            self.hours,
            self.gen_timings
        )
    }

    pub fn request_capture(&mut self) {
        self.r.capture_request = true;
    }

    /// 截圖完成時回傳 [寬(4 bytes), 高(4 bytes), RGBA...]，否則為空
    pub fn take_capture(&mut self) -> Vec<u8> {
        match self.r.take_capture() {
            Some((w, h, px)) => {
                let mut v = w.to_le_bytes().to_vec();
                v.extend_from_slice(&h.to_le_bytes());
                v.extend(px);
                v
            }
            None => Vec::new(),
        }
    }

    pub fn set_cam_mode_top(&mut self, height: f32) {
        self.top_view = height;
    }

    /// 測試用：把相機與車移到指定位置
    pub fn teleport(&mut self, x: f32, z: f32, heading: f32) {
        let y = self.fine.height_at(x, z);
        self.car.reset(Vec3::new(x, y + 1.0, z), heading);
        self.cam_yaw = heading;
        self.cam_pos = self.car.pos + Vec3::new(-heading.sin() * 6.0, 3.0, -heading.cos() * 6.0);
    }
}
