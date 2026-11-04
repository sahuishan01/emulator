use emulator_shader::{decode, emit, emit_normalized_position, Stage};
use std::collections::HashMap;

// Small independent SPIR-V value interpreter: validates actual generated data
// flow, including arithmetic using the original guest W before host projection.
fn execute(module: &[u32], input: [[u32; 4]; 2]) -> HashMap<u32, Vec<u32>> {
    let mut values: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut locations = HashMap::new();
    let mut stores = HashMap::new();
    let mut at = 5;
    while at < module.len() {
        let len = (module[at] >> 16) as usize;
        let op = module[at] & 65535;
        assert!(len > 0 && at + len <= module.len());
        let a = &module[at + 1..at + len];
        match op {
            71 if a[1] == 30 => {
                locations.insert(a[0], a[2]);
            }
            43 => {
                values.insert(a[1], vec![a[2]]);
            }
            61 => {
                values.insert(
                    a[1],
                    input[*locations.get(&a[2]).expect("input location") as usize].to_vec(),
                );
            }
            124 => {
                values.insert(a[1], values[&a[2]].clone());
            }
            81 => {
                values.insert(a[1], vec![values[&a[2]][a[3] as usize]]);
            }
            80 => {
                values.insert(a[1], a[2..].iter().map(|id| values[id][0]).collect());
            }
            129 | 133 | 136 => {
                let x = f32::from_bits(values[&a[2]][0]);
                let y = f32::from_bits(values[&a[3]][0]);
                values.insert(
                    a[1],
                    vec![match op {
                        129 => x + y,
                        133 => x * y,
                        _ => x / y,
                    }
                    .to_bits()],
                );
            }
            62 => {
                stores.insert(a[0], values[&a[1]].clone());
            }
            _ => {}
        }
        at += len;
    }
    stores
}

fn output(module: &[u32], builtin: bool) -> u32 {
    let mut outputs = std::collections::HashSet::new();
    let mut pos = 5;
    while pos < module.len() {
        let len = (module[pos] >> 16) as usize;
        let a = &module[pos + 1..pos + len];
        if module[pos] & 65535 == 59 && a[2] == 3 {
            outputs.insert(a[1]);
        }
        pos += len;
    }
    let mut at = 5;
    while at < module.len() {
        let len = (module[at] >> 16) as usize;
        let a = &module[at + 1..at + len];
        if module[at] & 65535 == 71
            && a.len() == 3
            && outputs.contains(&a[0])
            && a[1] == if builtin { 11 } else { 30 }
            && a[2] == 0
        {
            return a[0];
        }
        at += len;
    }
    panic!("Output decoration missing")
}

#[test]
fn post_guest_division_preserves_arithmetic_and_screen_linear_varyings() {
    for code in [
        include_bytes!("../../../examples/shaders/texture.vert.bin").as_slice(),
        include_bytes!("../../../examples/shaders/texture-transform.vert.bin").as_slice(),
        include_bytes!("../../../examples/shaders/passthrough.vert.bin").as_slice(),
    ] {
        let p = decode(code, Stage::Vertex).unwrap();
        let original = emit(&p).unwrap();
        let m = emit_normalized_position(&p).unwrap();
        assert_eq!(emit(&p).unwrap(), original); // Optional lowering cannot mutate the program/default module.
        for w in [0.25f32, 1., 2., 3., 16., 1.0 / 65536., 65536.] {
            for x in [-0.75f32, 0., 0.5] {
                let input = [
                    [x * w, 0.25 * w, 0.5 * w, w].map(f32::to_bits),
                    [-0.25f32, 1.25, 0., 1.].map(f32::to_bits),
                ];
                let guest = p.invoke(input, None).unwrap();
                let expected = guest.output.map(f32::from_bits);
                let actual = execute(&m, input);
                assert_eq!(
                    actual[&output(&m, true)],
                    vec![
                        (expected[0] / expected[3]).to_bits(),
                        (expected[1] / expected[3]).to_bits(),
                        (expected[2] / expected[3]).to_bits(),
                        1f32.to_bits()
                    ]
                );
                if let Some(varying) = guest.varying {
                    assert_eq!(actual[&output(&m, false)], varying.to_vec());
                }
            }
        }
    }
}

#[test]
fn fragment_stage_cannot_request_vertex_projection() {
    let p = decode(
        include_bytes!("../../../examples/shaders/green.frag.bin"),
        Stage::Fragment,
    )
    .unwrap();
    assert!(emit_normalized_position(&p).is_err());
}
