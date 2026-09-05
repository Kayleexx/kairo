(component
    (type $console-type (func (param "value" u32)))
    (import "console" (func $console (type $console-type)))
    (core func $console-lowered (canon lower (func $console)))
    (core module $guest
        (import "host" "console" (func $console (param i32)))
        (func (export "compute") (param i32) (result i32)
            local.get 0
            call $console
            local.get 0
            i32.const 2
            i32.mul))
    (core instance $host
        (export "console" (func $console-lowered)))
    (core instance $guest-instance
        (instantiate $guest (with "host" (instance $host))))
    (type $compute-type (func async (param "input" u32) (result u32)))
    (func $compute (type $compute-type)
        (canon lift (core func $guest-instance "compute")))
    (export "compute" (func $compute)))
