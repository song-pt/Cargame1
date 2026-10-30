use worldgen::{generate, validate, WorldSize, OFFICIAL_SEED};

fn now() -> f64 {
    0.0
}

#[test]
fn same_seed_same_world() {
    let a = generate(42, WorldSize::Small, &now);
    let b = generate(42, WorldSize::Small, &now);
    assert_eq!(a.fingerprint(), b.fingerprint());
}

#[test]
fn different_seed_different_world() {
    let a = generate(1, WorldSize::Small, &now);
    let b = generate(2, WorldSize::Small, &now);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn official_world_is_valid() {
    let w = generate(OFFICIAL_SEED, WorldSize::Medium, &now);
    let r = validate::check(&w);
    assert!(r.ok(), "{:?}", r.errors);
    assert!(r.land_km2 > 85.0 && r.land_km2 < 125.0, "陸地 {} km²", r.land_km2);
    assert!(r.tallest_m > 450.0, "CBD 地標高度 {}", r.tallest_m);
}

#[test]
fn several_seeds_are_valid() {
    for seed in [3u64, 7, 11] {
        let w = generate(seed, WorldSize::Small, &now);
        let r = validate::check(&w);
        assert!(r.ok(), "seed {}: {:?}", seed, r.errors);
    }
}
