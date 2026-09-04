(component
    (core module $probe
        (func (export "ping") (result i32)
            i32.const 42
        )
    )
    (core instance $probe-instance (instantiate $probe))
    (type $ping-type (func async (result u32)))
    (func $ping (type $ping-type)
        (canon lift (core func $probe-instance "ping"))
    )
    (export "ping" (func $ping))
)
