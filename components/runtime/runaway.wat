(component
    (core module $runaway
        (func (export "compute") (param i32) (result i32)
            (loop $spin
                br $spin)
            i32.const 0))
    (core instance $runaway-instance (instantiate $runaway))
    (type $compute-type (func async (param "input" u32) (result u32)))
    (func $compute (type $compute-type)
        (canon lift (core func $runaway-instance "compute")))
    (export "compute" (func $compute)))
