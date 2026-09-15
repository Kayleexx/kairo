(component
    (core module $stage
        (func (export "run") (param i32) (result i32)
            (local $i i32)
            (local $acc i32)
            local.get 0
            local.set $acc
            (block $done
                (loop $again
                    local.get $i
                    i32.const 20000000
                    i32.ge_u
                    br_if $done
                    local.get $acc
                    i32.const 1103515245
                    i32.mul
                    i32.const 12345
                    i32.add
                    local.set $acc
                    local.get $i
                    i32.const 1
                    i32.add
                    local.set $i
                    br $again))
            local.get $acc))
    (core instance $stage-instance (instantiate $stage))
    (type $run-type (func async (param "input" u32) (result u32)))
    (func $run (type $run-type)
        (canon lift (core func $stage-instance "run")))
    (export "run" (func $run)))
