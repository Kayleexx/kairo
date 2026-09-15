wit_bindgen::generate!({ world: "value-stage", path: "wit" });

struct Component;

impl Guest for Component {
    async fn run(_input: Vec<u8>) -> Result<Vec<u8>, String> {
        Err("value-poison always fails".to_owned())
    }
}

export!(Component);
