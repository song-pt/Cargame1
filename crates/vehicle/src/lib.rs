//! 車輛物理（偏模擬）：射線懸掛 + 簡化 Pacejka 輪胎 + 引擎扭矩曲線 + 變速箱。
//! 只使用基本浮點運算與 libm，保證重播的確定性。

use glam::{Mat3, Quat, Vec3};

pub const DT: f32 = 1.0 / 360.0;

pub trait Ground {
    /// 可行駛表面高度（含橋面）
    fn height(&self, x: f32, z: f32) -> f32;
    /// 摩擦係數倍率：柏油 1.0，草地約 0.7
    fn grip(&self, x: f32, z: f32) -> f32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    Rwd,
    Fwd,
    Awd,
}

/// 引擎直接由數值描述（玩家可調整），不做「換引擎」零件系統
#[derive(Clone, Debug)]
pub struct Engine {
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    pub peak_torque_nm: f32,
    pub peak_torque_rpm: f32,
}

impl Engine {
    /// 扭矩曲線：怠速 55% → 峰值 100% → 紅線 72%
    pub fn torque_at(&self, rpm: f32) -> f32 {
        let r = rpm.clamp(self.idle_rpm, self.redline_rpm);
        let shape = if r < self.peak_torque_rpm {
            let t = (r - self.idle_rpm) / (self.peak_torque_rpm - self.idle_rpm).max(1.0);
            0.55 + 0.45 * (1.0 - (1.0 - t) * (1.0 - t))
        } else {
            let t = (r - self.peak_torque_rpm) / (self.redline_rpm - self.peak_torque_rpm).max(1.0);
            1.0 - 0.28 * t * t
        };
        self.peak_torque_nm * shape
    }

    pub fn power_kw_at(&self, rpm: f32) -> f32 {
        self.torque_at(rpm) * rpm * core::f32::consts::TAU / 60.0 / 1000.0
    }
}

#[derive(Clone, Debug)]
pub struct CarParams {
    pub mass: f32,
    /// 車身尺寸（寬, 高, 長），用於慣量與車身碰撞
    pub body: Vec3,
    pub cg_height: f32,
    pub wheelbase: f32,
    pub track: f32,
    pub wheel_radius: f32,
    pub spring_rate: f32,
    pub damper: f32,
    pub rest_length: f32,
    pub max_travel: f32,
    pub drive: Drive,
    pub engine: Engine,
    /// 前進檔齒比（1 檔起）
    pub gears: Vec<f32>,
    pub reverse_ratio: f32,
    pub final_drive: f32,
    pub brake_torque: f32,
    /// 前輪制動分配
    pub brake_bias: f32,
    pub max_steer: f32,
    pub drag_area: f32,
    pub lift_area: f32,
    pub tire_mu: f32,
    pub fuel_capacity_l: f32,
}

impl CarParams {
    /// M0 測試車：前置後驅雙門跑車
    pub fn sports_coupe() -> Self {
        Self {
            mass: 1300.0,
            body: Vec3::new(1.82, 1.25, 4.35),
            cg_height: 0.48,
            wheelbase: 2.57,
            track: 1.54,
            wheel_radius: 0.33,
            spring_rate: 52_000.0,
            damper: 4_200.0,
            rest_length: 0.32,
            max_travel: 0.22,
            drive: Drive::Rwd,
            engine: Engine { idle_rpm: 900.0, redline_rpm: 7600.0, peak_torque_nm: 420.0, peak_torque_rpm: 4800.0 },
            gears: vec![3.6, 2.19, 1.54, 1.21, 1.0, 0.82],
            reverse_ratio: 3.4,
            final_drive: 3.9,
            brake_torque: 3_200.0,
            brake_bias: 0.62,
            max_steer: 0.58,
            drag_area: 0.68,
            lift_area: 0.25,
            tire_mu: 1.15,
            fuel_capacity_l: 55.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub throttle: f32,
    pub brake: f32,
    /// -1 左 … 1 右
    pub steer: f32,
    pub handbrake: bool,
    pub shift_up: bool,
    pub shift_down: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Wheel {
    pub omega: f32,
    pub compression: f32,
    pub contact: bool,
    pub steer: f32,
    pub load: f32,
    pub slip_ratio: f32,
    pub slip_angle: f32,
    pub on_road: f32,
}

#[derive(Clone, Debug)]
pub struct Car {
    pub p: CarParams,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub angvel: Vec3,
    pub wheels: [Wheel; 4],
    pub rpm: f32,
    /// -1 倒檔，0 空檔，1.. 前進檔
    pub gear: i32,
    pub automatic: bool,
    pub fuel_l: f32,
    pub fuel_enabled: bool,
    /// 循跡控制與 ABS（可在設定中關閉）
    pub tcs: bool,
    pub abs: bool,
    tcs_factor: f32,
    shift_timer: f32,
    steer_state: f32,
    reverse_timer: f32,
    inertia: Vec3,
}

impl Car {
    pub fn new(p: CarParams, pos: Vec3, heading: f32) -> Self {
        let (w, h, l) = (p.body.x, p.body.y, p.body.z);
        let m = p.mass;
        let inertia = Vec3::new(m * (h * h + l * l) / 12.0, m * (w * w + l * l) / 12.0, m * (w * w + h * h) / 12.0);
        let fuel = p.fuel_capacity_l;
        let mut c = Self {
            p,
            pos,
            rot: Quat::IDENTITY,
            vel: Vec3::ZERO,
            angvel: Vec3::ZERO,
            wheels: [Wheel::default(); 4],
            rpm: 900.0,
            gear: 1,
            automatic: false,
            fuel_l: fuel,
            fuel_enabled: true,
            tcs: true,
            abs: true,
            tcs_factor: 1.0,
            shift_timer: 0.0,
            steer_state: 0.0,
            reverse_timer: 0.0,
            inertia,
        };
        c.reset(pos, heading);
        c
    }

    pub fn reset(&mut self, pos: Vec3, heading: f32) {
        self.pos = pos;
        self.rot = Quat::from_rotation_y(heading);
        self.vel = Vec3::ZERO;
        self.angvel = Vec3::ZERO;
        self.gear = 1;
        for w in self.wheels.iter_mut() {
            w.omega = 0.0;
        }
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    /// 前進方向的有號速度
    pub fn forward_speed(&self) -> f32 {
        self.vel.dot(self.rot * Vec3::Z)
    }

    /// 車輪安裝點（車身局部座標）：0 左前，1 右前，2 左後，3 右後
    pub fn mount(&self, i: usize) -> Vec3 {
        let x = if i % 2 == 0 { -0.5 } else { 0.5 } * self.p.track;
        let z = if i < 2 { 0.5 } else { -0.5 } * self.p.wheelbase;
        Vec3::new(x, self.p.rest_length + self.p.wheel_radius - self.p.cg_height, z)
    }

    fn driven(&self, i: usize) -> bool {
        match self.p.drive {
            Drive::Rwd => i >= 2,
            Drive::Fwd => i < 2,
            Drive::Awd => true,
        }
    }

    fn ratio(&self) -> f32 {
        match self.gear {
            g if g > 0 => self.p.gears[(g - 1) as usize] * self.p.final_drive,
            g if g < 0 => -self.p.reverse_ratio * self.p.final_drive,
            _ => 0.0,
        }
    }

    pub fn step(&mut self, input: &Input, ground: &dyn Ground) {
        let dt = DT;
        let p = self.p.clone();
        let up = self.rot * Vec3::Y;
        let fwd_body = self.rot * Vec3::Z;
        let speed = self.speed();
        let vf = self.forward_speed();

        // 變速箱
        let max_gear = p.gears.len() as i32;
        if self.shift_timer > 0.0 {
            self.shift_timer -= dt;
        }
        let (mut throttle, mut brake) = (input.throttle.clamp(0.0, 1.0), input.brake.clamp(0.0, 1.0));
        // 自動換檔依車速計算的轉速，避免打滑時誤升檔
        let wheel_rpm_now = (vf / p.wheel_radius * self.ratio()).abs() * 60.0 / core::f32::consts::TAU;
        if self.automatic {
            if self.gear > 0 && self.shift_timer <= 0.0 {
                if wheel_rpm_now > p.engine.redline_rpm * 0.94 && self.gear < max_gear {
                    self.gear += 1;
                    self.shift_timer = 0.18;
                } else if self.gear > 1 && wheel_rpm_now < p.engine.redline_rpm * 0.42 {
                    self.gear -= 1;
                    self.shift_timer = 0.18;
                }
            }
            // 靜止時按剎車：切倒檔；倒檔時剎車鍵當油門
            if vf.abs() < 0.8 && brake > 0.3 && throttle < 0.05 {
                self.reverse_timer += dt;
                if self.reverse_timer > 0.35 && self.gear > 0 {
                    self.gear = -1;
                }
            } else {
                self.reverse_timer = 0.0;
            }
            if self.gear < 0 {
                if throttle > 0.05 && vf > -0.8 {
                    self.gear = 1;
                } else {
                    core::mem::swap(&mut throttle, &mut brake);
                }
            }
        } else {
            if input.shift_up && self.gear < max_gear {
                self.gear += 1;
                self.shift_timer = 0.12;
            }
            if input.shift_down && self.gear > -1 {
                if self.gear == 1 && vf > 2.0 {
                    // 行駛中不能直接進倒檔
                } else {
                    self.gear -= 1;
                    self.shift_timer = 0.12;
                }
            }
        }

        // 轉向：速度越高可用轉角越小，輸入帶速率限制
        let max_angle = p.max_steer * (1.0 / (1.0 + speed / 22.0)).max(0.22);
        let target = input.steer.clamp(-1.0, 1.0) * max_angle;
        let rate = 2.4 * dt;
        self.steer_state += (target - self.steer_state).clamp(-rate, rate);
        let toe = p.track / (2.0 * p.wheelbase);
        let st = self.steer_state;
        // 阿克曼：內側輪轉角較大
        let (sl, sr) = if st > 0.0 { (st * (1.0 - toe * st), st * (1.0 + toe * st)) } else { (st * (1.0 + toe * st.abs()), st * (1.0 - toe * st.abs())) };
        self.wheels[0].steer = sl;
        self.wheels[1].steer = sr;
        self.wheels[2].steer = 0.0;
        self.wheels[3].steer = 0.0;

        // 引擎
        let ratio = self.ratio();
        let n_driven = (0..4).filter(|&i| self.driven(i)).count() as f32;
        let wheel_omega = (0..4).filter(|&i| self.driven(i)).map(|i| self.wheels[i].omega).sum::<f32>() / n_driven;
        let coupled_rpm = (wheel_omega * ratio).abs() * 60.0 / core::f32::consts::TAU;
        let cutting = self.shift_timer > 0.0 || self.gear == 0;
        let has_fuel = !self.fuel_enabled || self.fuel_l > 0.0;
        let thr = if has_fuel { throttle } else { 0.0 };
        let launch_rpm = p.engine.idle_rpm + thr * 2600.0;
        let slipping = coupled_rpm < launch_rpm && !cutting;
        let target_rpm = if self.gear == 0 {
            p.engine.idle_rpm + thr * (p.engine.redline_rpm - p.engine.idle_rpm)
        } else if cutting {
            coupled_rpm.max(p.engine.idle_rpm)
        } else if slipping {
            launch_rpm
        } else {
            coupled_rpm
        };
        self.rpm += (target_rpm - self.rpm) * (dt * if cutting { 6.0 } else { 30.0 }).min(1.0);
        self.rpm = self.rpm.clamp(p.engine.idle_rpm * 0.8, p.engine.redline_rpm * 1.02);
        let limiter = if self.rpm >= p.engine.redline_rpm { 0.0 } else { 1.0 };
        let eng_torque = thr * p.engine.torque_at(self.rpm) * limiter
            - (1.0 - thr) * 0.12 * p.engine.peak_torque_nm * (self.rpm / p.engine.redline_rpm);
        // 離合器半聯動時只傳遞正扭矩（沒有引擎制動）
        let eng_torque = if slipping { eng_torque.max(0.0) } else { eng_torque };
        let mut drive_total = if cutting { 0.0 } else { eng_torque * ratio * 0.88 };
        // 循跡控制：驅動扭矩不超過輪胎能傳遞的上限，並在仍打滑時進一步收油
        if self.tcs {
            let cap: f32 = (0..4)
                .filter(|&i| self.driven(i))
                .map(|i| self.wheels[i].load * p.tire_mu * self.wheels[i].on_road.max(0.3) * p.wheel_radius)
                .sum::<f32>()
                * 1.08;
            let max_slip = (0..4).filter(|&i| self.driven(i) && self.wheels[i].contact).map(|i| (self.wheels[i].slip_ratio * ratio.signum()).max(0.0)).fold(0.0f32, f32::max);
            self.tcs_factor = if max_slip > 0.15 { (self.tcs_factor - 8.0 * dt).max(0.3) } else { (self.tcs_factor + 4.0 * dt).min(1.0) };
            if drive_total.abs() > cap && eng_torque > 0.0 {
                drive_total = drive_total.signum() * cap;
            }
            if eng_torque > 0.0 {
                drive_total *= self.tcs_factor;
            }
        }
        if self.fuel_enabled {
            let power = (thr * p.engine.torque_at(self.rpm) * self.rpm * core::f32::consts::TAU / 60.0).max(0.0);
            self.fuel_l = (self.fuel_l - (power / (0.3 * 34.2e6) + 0.00025) * dt).max(0.0);
        }

        // 車輪
        let mut force = Vec3::new(0.0, -9.81 * p.mass, 0.0);
        let mut torque = Vec3::ZERO;
        let iw = 1.1f32;
        let driven: [bool; 4] = [self.driven(0), self.driven(1), self.driven(2), self.driven(3)];
        let (pos, rot, vel, angvel) = (self.pos, self.rot, self.vel, self.angvel);
        for i in 0..4 {
            let mount = pos + rot * self.mount(i);
            let gh = ground.height(mount.x, mount.z);
            let w = &mut self.wheels[i];
            if up.y < 0.25 {
                w.contact = false;
                continue;
            }
            let t = (mount.y - gh) / up.y;
            let len = t - p.wheel_radius;
            let prev = w.compression;
            if len >= p.rest_length {
                w.contact = false;
                w.compression = 0.0;
                w.load = 0.0;
                // 空中車輪仍受驅動與剎車
                let mut om = w.omega + (if driven[i] { drive_total / n_driven } else { 0.0 }) / iw * dt;
                let bt = brake * p.brake_torque * if i < 2 { p.brake_bias } else { 1.0 - p.brake_bias } * 2.0 + if input.handbrake && i >= 2 { 4000.0 } else { 0.0 };
                let db = bt / iw * dt;
                om = if om.abs() <= db { 0.0 } else { om - om.signum() * db };
                w.omega = om;
                continue;
            }
            w.contact = true;
            let comp = (p.rest_length - len).min(p.max_travel + 0.05);
            w.compression = comp;
            let cvel = (comp - prev) / dt;
            let fs = (p.spring_rate * comp + p.damper * cvel).max(0.0);
            // 觸底：額外硬彈簧
            let fs = fs + if comp > p.max_travel { (comp - p.max_travel) * p.spring_rate * 8.0 } else { 0.0 };
            w.load = fs;
            let cp = mount - up * t;
            let e = 0.4;
            let n = Vec3::new(ground.height(cp.x - e, cp.z) - ground.height(cp.x + e, cp.z), 2.0 * e, ground.height(cp.x, cp.z - e) - ground.height(cp.x, cp.z + e)).normalize();
            let (ss, cs) = (libm::sinf(w.steer), libm::cosf(w.steer));
            let fwd_local = Vec3::new(ss, 0.0, cs);
            let mut f = rot * fwd_local;
            f = (f - n * f.dot(n)).normalize_or_zero();
            let right = n.cross(f);
            let r = cp - pos;
            let vc = vel + angvel.cross(r);
            let vx = vc.dot(f);
            let vy = vc.dot(right);
            let grip = ground.grip(cp.x, cp.z);
            w.on_road = grip;
            let mu = p.tire_mu * grip;
            let fz = fs.min(p.mass * 9.81 * 1.6);

            // 側向：魔術公式
            let denom = vx.abs().max(4.0);
            let alpha = libm::atanf(vy / denom);
            w.slip_angle = alpha;
            let fy_raw = -mu * fz * libm::sinf(1.9 * libm::atanf(9.0 * alpha));

            // 縱向：車輪角速度子步積分
            let drive_t = if driven[i] { drive_total / n_driven } else { 0.0 };
            let bias = if i < 2 { p.brake_bias } else { 1.0 - p.brake_bias };
            let mut bt = brake * p.brake_torque * bias * 2.0;
            // ABS：鎖死趨勢時放鬆剎車
            if self.abs && w.slip_ratio < -0.15 && vx.abs() > 3.0 {
                bt *= 0.35;
            }
            if input.handbrake && i >= 2 {
                bt += 4000.0;
            }
            let sub = 4;
            let hdt = dt / sub as f32;
            let mut fx_acc = 0.0;
            for _ in 0..sub {
                let kappa = (w.omega * p.wheel_radius - vx) / denom;
                let fx = mu * fz * libm::sinf(1.65 * libm::atanf(11.0 * kappa));
                let mut om = w.omega + (drive_t - fx * p.wheel_radius) / iw * hdt;
                let db = bt / iw * hdt;
                om = if om.abs() <= db { 0.0 } else { om - om.signum() * db };
                w.omega = om;
                fx_acc += fx;
            }
            let mut fx = fx_acc / sub as f32;
            w.slip_ratio = (w.omega * p.wheel_radius - vx) / denom;
            // 摩擦圓
            let mut fy = fy_raw;
            let total = libm::sqrtf(fx * fx + fy * fy);
            let lim = mu * fz;
            if total > lim && total > 0.0 {
                let k = lim / total;
                fx *= k;
                fy *= k;
            }
            // 低速時抑制側滑抖動
            if vx.abs() < 2.0 {
                let fy_low = (-vy * p.mass * 0.25 / dt * 0.02).clamp(-lim, lim);
                let t = vx.abs() / 2.0;
                fy = fy * t + fy_low * (1.0 - t);
            }
            let tire = f * fx + right * fy;
            let susp = up * fs;
            force += tire + susp;
            torque += r.cross(tire + susp);
        }

        // 空氣動力
        let v2 = speed * speed;
        force += -self.vel * (0.5 * 1.225 * p.drag_area * speed);
        force += -up * (0.5 * 1.225 * p.lift_area * v2);

        // 車身碰撞（八個角）
        let hb = p.body * 0.5;
        for cx in [-1.0f32, 1.0] {
            for cy in [-0.2f32, 1.0] {
                for cz in [-1.0f32, 1.0] {
                    let local = Vec3::new(cx * hb.x, cy * hb.y + (hb.y - p.cg_height) * 0.0, cz * hb.z);
                    let wp = self.pos + self.rot * local;
                    let gh = ground.height(wp.x, wp.z);
                    let pen = gh - wp.y;
                    if pen > 0.0 {
                        let r = wp - self.pos;
                        let vc = self.vel + self.angvel.cross(r);
                        let fnrm = (pen * p.mass * 600.0 - vc.y * p.mass * 12.0).max(0.0);
                        let vt = Vec3::new(vc.x, 0.0, vc.z);
                        let ff = -vt.normalize_or_zero() * (fnrm * 0.6).min(vt.length() * p.mass * 20.0);
                        let fc = Vec3::new(0.0, fnrm, 0.0) + ff;
                        force += fc;
                        torque += r.cross(fc);
                    }
                }
            }
        }

        // 積分
        self.vel += force / p.mass * dt;
        let rm = Mat3::from_quat(self.rot);
        let local_t = rm.transpose() * torque;
        let local_w = rm.transpose() * self.angvel;
        let i = self.inertia;
        // 歐拉方程（含陀螺項）
        let gyro = local_w.cross(local_w * i);
        let dw = (local_t - gyro) / i;
        self.angvel = rm * (local_w + dw * dt);
        self.angvel *= 1.0 - 0.15 * dt;
        self.pos += self.vel * dt;
        let wq = Quat::from_xyzw(self.angvel.x, self.angvel.y, self.angvel.z, 0.0);
        self.rot = (self.rot + wq * self.rot * (0.5 * dt)).normalize();
        let _ = fwd_body;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat;
    impl Ground for Flat {
        fn height(&self, _: f32, _: f32) -> f32 {
            0.0
        }
        fn grip(&self, _: f32, _: f32) -> f32 {
            1.0
        }
    }

    fn settle(car: &mut Car) {
        for _ in 0..720 {
            car.step(&Input::default(), &Flat);
        }
    }

    #[test]
    fn rests_on_ground() {
        let mut car = Car::new(CarParams::sports_coupe(), Vec3::new(0.0, 1.0, 0.0), 0.0);
        settle(&mut car);
        assert!(car.speed() < 0.05, "靜止速度 {}", car.speed());
        assert!(car.pos.y > 0.2 && car.pos.y < 0.8, "車身高度 {}", car.pos.y);
        assert!((car.rot * Vec3::Y).y > 0.999);
    }

    #[test]
    fn accelerates_and_brakes() {
        let mut car = Car::new(CarParams::sports_coupe(), Vec3::new(0.0, 1.0, 0.0), 0.0);
        car.automatic = true;
        settle(&mut car);
        let thr = Input { throttle: 1.0, ..Default::default() };
        let mut t100 = None;
        for i in 0..(360 * 20) {
            car.step(&thr, &Flat);
            if t100.is_none() && car.forward_speed() * 3.6 >= 100.0 {
                t100 = Some(i as f32 * DT);
            }
        }
        let t = t100.expect("20 秒內未到 100 km/h");
        assert!(t > 3.0 && t < 9.0, "0-100 km/h {:.1} s", t);
        assert!(car.forward_speed() * 3.6 > 160.0, "20 秒後 {:.0} km/h", car.forward_speed() * 3.6);
        let brk = Input { brake: 1.0, ..Default::default() };
        let mut dist = 0.0;
        let v0 = car.forward_speed();
        while car.forward_speed() > 0.5 {
            car.step(&brk, &Flat);
            dist += car.forward_speed() * DT;
        }
        // 從高速剎停的平均減速度應在 0.8g 左右
        let decel = v0 * v0 / (2.0 * dist) / 9.81;
        assert!(decel > 0.6 && decel < 1.4, "平均減速度 {:.2} g", decel);
    }

    #[test]
    fn turns() {
        let mut car = Car::new(CarParams::sports_coupe(), Vec3::new(0.0, 1.0, 0.0), 0.0);
        car.automatic = true;
        settle(&mut car);
        for _ in 0..(360 * 4) {
            car.step(&Input { throttle: 0.6, ..Default::default() }, &Flat);
        }
        let h0 = (car.rot * Vec3::Z).x;
        for _ in 0..(360 * 2) {
            car.step(&Input { throttle: 0.3, steer: 1.0, ..Default::default() }, &Flat);
        }
        let f = car.rot * Vec3::Z;
        assert!((f.x - h0).abs() > 0.3, "沒有轉向");
        assert!((car.rot * Vec3::Y).y > 0.9, "翻車了");
    }

    #[test]
    fn deterministic() {
        let run = || {
            let mut car = Car::new(CarParams::sports_coupe(), Vec3::new(0.0, 1.0, 0.0), 0.3);
            car.automatic = true;
            for i in 0..2000 {
                let steer = if (i / 300) % 2 == 0 { 0.7 } else { -0.5 };
                car.step(&Input { throttle: 0.8, steer, ..Default::default() }, &Flat);
            }
            (car.pos, car.rot)
        };
        assert_eq!(run(), run());
    }
}
