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
            (local $records i32) (local $total i32) (local $number i32)
            (local $in-number i32) (local $in-string i32)
            (local $depth i32) (local $has-number i32)
            (block $done
                (loop $read-more
                    local.get $stream i32.const 0 i32.const 65536 call $read local.set $status
                    local.get $status i32.const -1 i32.eq if unreachable end
                    i32.const 0 local.set $index
                    (block $chunk-done
                        (loop $bytes
                            local.get $index local.get $status i32.const 4 i32.shr_u i32.ge_u br_if $chunk-done
                            local.get $index i32.load8_u local.tee $byte i32.const 34 i32.eq
                            if
                                local.get $in-string i32.eqz local.set $in-string
                            else
                                local.get $in-string i32.eqz
                                if
                                    local.get $byte i32.const 123 i32.eq
                                    if
                                        local.get $depth i32.eqz i32.eqz if unreachable end
                                        i32.const 1 local.set $depth
                                    end
                                    local.get $byte i32.const 125 i32.eq
                                    if
                                        local.get $depth i32.const 1 i32.ne if unreachable end
                                        i32.const 0 local.set $depth
                                    end
                                    local.get $byte i32.const 48 i32.ge_u
                                    local.get $byte i32.const 57 i32.le_u i32.and
                                    if
                                        local.get $number i32.const 10 i32.mul
                                        local.get $byte i32.const 48 i32.sub i32.add local.set $number
                                        i32.const 1 local.set $in-number
                                        i32.const 1 local.set $has-number
                                    else
                                        local.get $in-number
                                        if
                                            local.get $total local.get $number i32.add local.set $total
                                            i32.const 0 local.set $number i32.const 0 local.set $in-number
                                        end
                                    end
                                end
                                local.get $byte i32.const 10 i32.eq
                                if
                                    local.get $depth i32.eqz i32.eqz if unreachable end
                                    local.get $has-number i32.eqz if unreachable end
                                    local.get $records i32.const 1 i32.add local.set $records
                                    i32.const 0 local.set $has-number
                                end
                            end
                            local.get $index i32.const 1 i32.add local.set $index
                            br $bytes))
                    local.get $status i32.const 1 i32.and br_if $done
                    br $read-more))
            local.get $in-number if local.get $total local.get $number i32.add local.set $total end
            local.get $depth i32.eqz i32.eqz if unreachable end
            local.get $in-string if unreachable end
            local.get $records i64.extend_i32_u i64.const 32 i64.shl
            local.get $total i64.extend_i32_u i64.or))
    (core instance $consume-instance
        (instantiate $consume (with "" (instance
            (export "memory" (memory $libc "memory"))
            (export "stream.read" (func $stream.read))))))
    (func (export "consume") async (param "input" $bytes) (result u64)
        (canon lift (core func $consume-instance "consume")
            (memory (core memory $libc "memory")))))
