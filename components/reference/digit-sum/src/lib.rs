wit_bindgen::generate!({ world: "value-stage", path: "wit" });

struct Component;

impl Guest for Component {
    async fn run(input: Vec<u8>) -> Result<Vec<u8>, String> {
        let text = std::str::from_utf8(&input).map_err(|error| error.to_string())?;
        let value: u64 = text
            .trim()
            .parse()
            .map_err(|error: std::num::ParseIntError| error.to_string())?;
        let sum: u32 = value
            .to_string()
            .bytes()
            .map(|digit| u32::from(digit - b'0'))
            .sum();
        Ok(sum.to_string().into_bytes())
    }
}

export!(Component);
