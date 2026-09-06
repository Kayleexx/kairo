(component
    (type $bytes (stream u8))
    (core func $stream.drop-readable (canon stream.drop-readable $bytes))
    (core module $consume
        (import "" "stream.drop-readable" (func $stream.drop-readable (param i32)))
        (func (export "consume") (param i32) (result i64)
            local.get 0
            call $stream.drop-readable
            i64.const 0))
    (core instance $consume-instance
        (instantiate $consume
            (with "" (instance
                (export "stream.drop-readable" (func $stream.drop-readable))))))
    (func (export "consume") async (param "input" $bytes) (result u64)
        (canon lift (core func $consume-instance "consume"))))
