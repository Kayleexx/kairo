(component
    (core module $memory-limit
        (memory 1025)
        (func (export "compute") (param i32) (result i32)
            i32.const 0))
    (core instance $memory-limit-instance (instantiate $memory-limit))
    (type $compute-type (func async (param "input" u32) (result u32)))
    (func $compute (type $compute-type)
        (canon lift (core func $memory-limit-instance "compute")))
    (export "compute" (func $compute)))
