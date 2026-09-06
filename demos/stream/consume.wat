(component
    (core module $libc
        (memory (export "memory") 1))
    (core instance $libc (instantiate $libc))
    (type $bytes (stream u8))
    (core func $stream.read
        (canon stream.read $bytes async (memory (core memory $libc "memory"))))
    (core module $consume
        (import "" "memory" (memory 1))
        (import "" "stream.read" (func $stream.read (param i32 i32 i32) (result i32)))
        (func (export "consume") (param $stream i32) (result i64)
            (local $status i32)
            (local $index i32)
            (local $checksum i32)
            (local $bytes i64)
            (block $done
                (loop $read
                    local.get $stream
                    i32.const 0
                    i32.const 65536
                    call $stream.read
                    local.tee $status
                    i32.const -1
                    i32.eq
                    if unreachable end
                    local.get $status
                    i32.const 4
                    i32.shr_u
                    local.tee $index
                    i64.extend_i32_u
                    local.get $bytes
                    i64.add
                    local.set $bytes
                    i32.const 0
                    local.set $index
                    (block $chunk_done
                        (loop $checksum_bytes
                            local.get $index
                            local.get $status
                            i32.const 4
                            i32.shr_u
                            i32.ge_u
                            br_if $chunk_done
                            local.get $checksum
                            local.get $index
                            i32.load8_u
                            i32.add
                            local.set $checksum
                            local.get $index
                            i32.const 1
                            i32.add
                            local.set $index
                            br $checksum_bytes))
                    local.get $status
                    i32.const 1
                    i32.and
                    br_if $done
                    br $read))
            local.get $bytes
            i64.const 32
            i64.shl
            local.get $checksum
            i64.extend_i32_u
            i64.or))
    (core instance $consume-instance
        (instantiate $consume
            (with "" (instance
                (export "memory" (memory $libc "memory"))
                (export "stream.read" (func $stream.read))))))
    (func (export "consume") async (param "input" $bytes) (result u64)
        (canon lift (core func $consume-instance "consume")
            (memory (core memory $libc "memory")))))
