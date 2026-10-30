use vehicle::*;
use glam::Vec3;
struct Flat;
impl Ground for Flat { fn height(&self,_:f32,_:f32)->f32{0.0} fn grip(&self,_:f32,_:f32)->f32{1.0} }
fn main(){
    let mut car = Car::new(CarParams::sports_coupe(), Vec3::new(0.0,1.0,0.0), 0.0);
    car.automatic = true;
    for _ in 0..720 { car.step(&Input::default(), &Flat); }
    for i in 0..(360*14) {
        car.step(&Input{throttle:1.0,..Default::default()}, &Flat);
        if i % 180 == 0 {
            let w=&car.wheels;
            println!("t={:.1} v={:.1}km/h gear={} rpm={:.0} slipR={:.2} loadR={:.0} loadF={:.0} omegaR={:.1} y={:.2}", i as f32*DT, car.forward_speed()*3.6, car.gear, car.rpm, w[2].slip_ratio, w[2].load, w[0].load, w[2].omega, car.pos.y);
        }
    }
}
