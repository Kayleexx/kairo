wit_bindgen::generate!({ world: "value-stage", path: "wit" });

// bounds the sieve's memory (one byte per candidate) against an absurd or malicious input.
const MAX_UPPER_BOUND: u64 = 50_000_000;

struct Component;

impl Guest for Component {
    async fn run(input: Vec<u8>) -> Result<Vec<u8>, String> {
        let text = std::str::from_utf8(&input).map_err(|error| error.to_string())?;
        let upper_bound: u64 = text
            .trim()
            .parse()
            .map_err(|error: std::num::ParseIntError| error.to_string())?;
        if upper_bound > MAX_UPPER_BOUND {
            return Err(format!(
                "upper bound {upper_bound} exceeds the {MAX_UPPER_BOUND} limit"
            ));
        }
        Ok(count_primes(upper_bound).to_string().into_bytes())
    }
}

fn count_primes(upper_bound: u64) -> u64 {
    if upper_bound < 2 {
        return 0;
    }
    let upper_bound = upper_bound as usize;
    let mut sieve = vec![true; upper_bound + 1];
    sieve[0] = false;
    sieve[1] = false;
    let mut candidate = 2;
    while candidate * candidate <= upper_bound {
        if sieve[candidate] {
            let mut multiple = candidate * candidate;
            while multiple <= upper_bound {
                sieve[multiple] = false;
                multiple += candidate;
            }
        }
        candidate += 1;
    }
    sieve.into_iter().filter(|&is_prime| is_prime).count() as u64
}

export!(Component);
