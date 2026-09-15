wit_bindgen::generate!({ world: "value-stage", path: "wit" });

struct Component;

impl Guest for Component {
    async fn run(input: Vec<u8>) -> Result<Vec<u8>, String> {
        // real CPU-bound work (not a sleep) to give tests a real kill window.
        let mut count: u32 = 0;
        for candidate in 2..2_000_000u32 {
            if is_prime(candidate) {
                count += 1;
            }
        }
        if count == 0 {
            return Err("unreachable: no primes found".to_owned());
        }
        Ok(input.into_iter().map(|byte| byte.wrapping_add(1)).collect())
    }
}

fn is_prime(value: u32) -> bool {
    if value < 2 {
        return false;
    }
    let mut divisor = 2;
    while divisor * divisor <= value {
        if value % divisor == 0 {
            return false;
        }
        divisor += 1;
    }
    true
}

export!(Component);
