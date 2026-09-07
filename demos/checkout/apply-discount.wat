(component
    (core module $stage
        (func (export "run") (param i32) (result i32)
            local.get 0
            i32.const 90
            i32.mul
            i32.const 100
            i32.div_u))
    (core instance $stage-instance (instantiate $stage))
    (type $run-type (func async (param "input" u32) (result u32)))
    (func $run (type $run-type)
        (canon lift (core func $stage-instance "run")))
    (export "run" (func $run)))
