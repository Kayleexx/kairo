wit_bindgen::generate!({ world: "value-stage", path: "wit" });

struct Component;

impl Guest for Component {
    async fn run(input: Vec<u8>) -> Result<Vec<u8>, String> {
        if input.is_empty() {
            return Err("input must not be empty".to_owned());
        }
        Ok(input.into_iter().map(|byte| byte.wrapping_add(1)).collect())
    }
}

export!(Component);
