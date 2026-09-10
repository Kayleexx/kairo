(component
    (core module $libc (memory (export "memory") 1))
    (core instance $libc (instantiate $libc))
    (type $bytes (stream u8))
    (core func $stream.read
        (canon stream.read $bytes async (memory (core memory $libc "memory"))))
    (core module $consume
        (import "" "memory" (memory 1))
        (import "" "stream.read" (func $read (param i32 i32 i32) (result i32)))
        (func (export "consume") (param $stream i32) (result i64)
            (local $status i32) (local $index i32) (local $byte i32)
            (local $line i32) (local $frames i32) (local $pixels i32) (local $sum i64)
            (local $header-count i32) (local $header-signature i64)
            (local $frame-count i32) (local $frame-signature i64)
            (block $done
                (loop $read-more
                    local.get $stream i32.const 0 i32.const 65536 call $read local.set $status
                    local.get $status i32.const -1 i32.eq if unreachable end
                    i32.const 0 local.set $index
                    (block $chunk-done
                        (loop $bytes
                            local.get $index local.get $status i32.const 4 i32.shr_u i32.ge_u br_if $chunk-done
                            local.get $index i32.load8_u local.set $byte
                            local.get $line i32.const 2 i32.lt_u
                            if
                                local.get $byte i32.const 10 i32.eq
                                if
                                    local.get $line i32.eqz
                                    if
                                        local.get $header-count i32.const 9 i32.ne if unreachable end
                                        local.get $header-signature i64.const 2990904146172123795 i64.ne if unreachable end
                                    else
                                        local.get $frame-count i32.const 5 i32.ne if unreachable end
                                        local.get $frame-signature i64.const 306769157739 i64.ne if unreachable end
                                    end
                                    local.get $line i32.const 1 i32.eq
                                    if local.get $frames i32.const 1 i32.add local.set $frames end
                                    local.get $line i32.const 1 i32.add local.set $line
                                else
                                    local.get $line i32.eqz
                                    if
                                        local.get $header-count i32.const 9 i32.lt_u
                                        if
                                            local.get $header-signature i64.const 257 i64.mul
                                            local.get $byte i64.extend_i32_u i64.add local.set $header-signature
                                            local.get $header-count i32.const 1 i32.add local.set $header-count
                                        end
                                    else
                                        local.get $frame-signature i64.const 257 i64.mul
                                        local.get $byte i64.extend_i32_u i64.add local.set $frame-signature
                                        local.get $frame-count i32.const 1 i32.add local.set $frame-count
                                    end
                                end
                            else
                                local.get $sum local.get $byte i64.extend_i32_u i64.add local.set $sum
                                local.get $pixels i32.const 1 i32.add local.set $pixels
                            end
                            local.get $index i32.const 1 i32.add local.set $index
                            br $bytes))
                    local.get $status i32.const 1 i32.and br_if $done
                    br $read-more))
            local.get $frames i32.eqz if unreachable end
            local.get $pixels i32.eqz if unreachable end
            local.get $frames i64.extend_i32_u i64.const 32 i64.shl
            local.get $sum local.get $pixels i64.extend_i32_u i64.div_u i64.or))
    (core instance $consume-instance
        (instantiate $consume (with "" (instance
            (export "memory" (memory $libc "memory"))
            (export "stream.read" (func $stream.read))))))
    (func (export "consume") async (param "input" $bytes) (result u64)
        (canon lift (core func $consume-instance "consume")
            (memory (core memory $libc "memory")))))
