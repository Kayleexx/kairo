(component
    (type $console-type (func (param "value" u32)))
    (import "console" (func $console (type $console-type)))
    (core func $console-lowered (canon lower (func $console)))
    (core module $guest
        (import "host" "console" (func $console (param i32)))
        (func (export "run") (param i32) (result i32)
            (local $index i32)
            (block $done
                (loop $write
                    local.get $index
                    i32.const 500000
                    i32.ge_u
                    br_if $done
                    local.get 0
                    call $console
                    local.get $index
                    i32.const 1
                    i32.add
                    local.set $index
                    br $write))
            local.get 0))
    (core instance $host
        (export "console" (func $console-lowered)))
    (core instance $guest
        (instantiate $guest (with "host" (instance $host))))
    (type $run-type (func async (param "input" u32) (result u32)))
    (func $run (type $run-type)
        (canon lift (core func $guest "run")))
    (export "run" (func $run)))
