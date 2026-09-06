(component
    (type $bytes (stream u8))
    (core module $transform
        (func (export "transform") (param i32) (result i32)
            local.get 0))
    (core instance $transform-instance (instantiate $transform))
    (func (export "transform") async (param "input" $bytes) (result $bytes)
        (canon lift (core func $transform-instance "transform"))))
