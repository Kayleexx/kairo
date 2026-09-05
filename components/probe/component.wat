(component
    (core module $probe
        (func (export "compute") (param i32) (result i32)
            local.get 0
            i32.const 2
            i32.mul
        )
    )
    (core instance $probe-instance (instantiate $probe))
    (type $compute-type (func async (param "input" u32) (result u32)))
    (func $compute (type $compute-type)
        (canon lift (core func $probe-instance "compute"))
    )
    (export "compute" (func $compute))
)
