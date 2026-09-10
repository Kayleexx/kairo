(component
    (core module $libc (memory (export "memory") 2))
    (core instance $libc (instantiate $libc))
    (type $bytes (stream u8))
    (core func $stream.read
        (canon stream.read $bytes async (memory (core memory $libc "memory"))))
    (core module $consume
        (import "" "memory" (memory 2))
        (import "" "stream.read" (func $read (param i32 i32 i32) (result i32)))
        (data (i32.const 1024) "YUV4MPEG2")
        (data (i32.const 1033) "FRAME")
        (data (i32.const 1038) "mono")
        (data (i32.const 1042) "420")
        (data (i32.const 1045) "420jpeg")
        (data (i32.const 1052) "420mpeg2")
        (data (i32.const 1060) "420paldv")
        (global $width (mut i32) (i32.const 0))
        (global $height (mut i32) (i32.const 0))
        (global $chroma (mut i32) (i32.const 1))
        (global $luma-size (mut i32) (i32.const 0))
        (global $frame-size (mut i32) (i32.const 0))

        (func $equal (param $start i32) (param $length i32)
                     (param $expected i32) (param $expected-length i32) (result i32)
            (local $i i32)
            local.get $length local.get $expected-length i32.ne
            if i32.const 0 return end
            (block $done
                (loop $next
                    local.get $i local.get $length i32.ge_u br_if $done
                    local.get $start local.get $i i32.add i32.load8_u
                    local.get $expected local.get $i i32.add i32.load8_u
                    i32.ne if i32.const 0 return end
                    local.get $i i32.const 1 i32.add local.set $i
                    br $next))
            i32.const 1)

        (func $decimal (param $start i32) (param $end i32) (result i32)
            (local $value i32) (local $byte i32)
            local.get $start local.get $end i32.ge_u if unreachable end
            (block $done
                (loop $next
                    local.get $start local.get $end i32.ge_u br_if $done
                    local.get $start i32.load8_u local.tee $byte
                    i32.const 48 i32.lt_u if unreachable end
                    local.get $byte i32.const 57 i32.gt_u if unreachable end
                    local.get $value i32.const 409 i32.gt_u if unreachable end
                    local.get $value i32.const 10 i32.mul
                    local.get $byte i32.const 48 i32.sub i32.add local.set $value
                    local.get $start i32.const 1 i32.add local.set $start
                    br $next))
            local.get $value i32.eqz if unreachable end
            local.get $value i32.const 4096 i32.gt_u if unreachable end
            local.get $value)

        (func $parse-file-header (param $length i32)
            (local $start i32) (local $end i32) (local $kind i32) (local $luma i32)
            i32.const 0 i32.const 9 i32.const 1024 i32.const 9 call $equal
            i32.eqz if unreachable end
            local.get $length i32.const 11 i32.lt_u if unreachable end
            i32.const 9 i32.load8_u i32.const 32 i32.ne if unreachable end
            i32.const 10 local.set $start
            (block $done
                (loop $token
                    local.get $start local.get $length i32.ge_u br_if $done
                    local.get $start local.set $end
                    (block $token-end
                        (loop $scan
                            local.get $end local.get $length i32.ge_u br_if $token-end
                            local.get $end i32.load8_u i32.const 32 i32.eq br_if $token-end
                            local.get $end i32.const 1 i32.add local.set $end
                            br $scan))
                    local.get $end local.get $start i32.eq if unreachable end
                    local.get $start i32.load8_u local.set $kind
                    local.get $kind i32.const 87 i32.eq
                    if
                        local.get $start i32.const 1 i32.add local.get $end call $decimal
                        global.set $width
                    else
                        local.get $kind i32.const 72 i32.eq
                        if
                            local.get $start i32.const 1 i32.add local.get $end call $decimal
                            global.set $height
                        else
                            local.get $kind i32.const 67 i32.eq
                            if
                                local.get $start i32.const 1 i32.add
                                local.get $end local.get $start i32.sub i32.const 1 i32.sub
                                i32.const 1038 i32.const 4 call $equal
                                if
                                    i32.const 0 global.set $chroma
                                else
                                    local.get $start i32.const 1 i32.add
                                    local.get $end local.get $start i32.sub i32.const 1 i32.sub
                                    i32.const 1042 i32.const 3 call $equal
                                    local.get $start i32.const 1 i32.add
                                    local.get $end local.get $start i32.sub i32.const 1 i32.sub
                                    i32.const 1045 i32.const 7 call $equal i32.or
                                    local.get $start i32.const 1 i32.add
                                    local.get $end local.get $start i32.sub i32.const 1 i32.sub
                                    i32.const 1052 i32.const 8 call $equal i32.or
                                    local.get $start i32.const 1 i32.add
                                    local.get $end local.get $start i32.sub i32.const 1 i32.sub
                                    i32.const 1060 i32.const 8 call $equal i32.or
                                    i32.eqz if unreachable end
                                    i32.const 1 global.set $chroma
                                end
                            end
                        end
                    end
                    local.get $end i32.const 1 i32.add local.set $start
                    br $token))
            global.get $width global.get $height i32.mul local.tee $luma
            global.set $luma-size
            global.get $chroma
            if
                global.get $width i32.const 1 i32.add i32.const 2 i32.div_u
                global.get $height i32.const 1 i32.add i32.const 2 i32.div_u
                i32.mul i32.const 2 i32.mul local.get $luma i32.add
                global.set $frame-size
            else
                local.get $luma global.set $frame-size
            end)

        (func $parse-frame-header (param $length i32)
            local.get $length i32.const 5 i32.lt_u if unreachable end
            i32.const 0 i32.const 5 i32.const 1033 i32.const 5 call $equal
            i32.eqz if unreachable end
            local.get $length i32.const 5 i32.gt_u
            if i32.const 5 i32.load8_u i32.const 32 i32.ne if unreachable end end)

        (func (export "consume") (param $stream i32) (result i64)
            (local $status i32) (local $index i32) (local $byte i32)
            (local $state i32) (local $header-length i32) (local $payload i32)
            (local $frames i32) (local $pixels i64) (local $sum i64)
            (block $done
                (loop $read-more
                    local.get $stream i32.const 4096 i32.const 61440 call $read local.set $status
                    local.get $status i32.const -1 i32.eq if unreachable end
                    i32.const 0 local.set $index
                    (block $chunk-done
                        (loop $bytes
                            local.get $index local.get $status i32.const 4 i32.shr_u i32.ge_u
                            br_if $chunk-done
                            local.get $index i32.const 4096 i32.add i32.load8_u local.set $byte
                            local.get $state i32.const 2 i32.lt_u
                            if
                                local.get $byte i32.const 10 i32.eq
                                if
                                    local.get $state i32.eqz
                                    if
                                        local.get $header-length call $parse-file-header
                                        i32.const 1 local.set $state
                                    else
                                        local.get $header-length call $parse-frame-header
                                        i32.const 2 local.set $state
                                        i32.const 0 local.set $payload
                                    end
                                    i32.const 0 local.set $header-length
                                else
                                    local.get $header-length i32.const 1024 i32.ge_u
                                    if unreachable end
                                    local.get $header-length local.get $byte i32.store8
                                    local.get $header-length i32.const 1 i32.add local.set $header-length
                                end
                            else
                                local.get $state i32.const 2 i32.eq
                                if
                                    local.get $sum local.get $byte i64.extend_i32_u i64.add local.set $sum
                                    local.get $pixels i64.const 1 i64.add local.set $pixels
                                end
                                local.get $payload i32.const 1 i32.add local.tee $payload
                                global.get $luma-size i32.eq
                                if i32.const 3 local.set $state end
                                local.get $payload global.get $frame-size i32.eq
                                if
                                    local.get $frames i32.const 1 i32.add local.set $frames
                                    i32.const 1 local.set $state
                                    i32.const 0 local.set $payload
                                end
                            end
                            local.get $index i32.const 1 i32.add local.set $index
                            br $bytes))
                    local.get $status i32.const 1 i32.and br_if $done
                    br $read-more))
            local.get $state i32.const 1 i32.ne if unreachable end
            local.get $header-length i32.eqz i32.eqz if unreachable end
            local.get $frames i32.eqz if unreachable end
            local.get $pixels i64.eqz if unreachable end
            local.get $frames i64.extend_i32_u i64.const 32 i64.shl
            local.get $sum local.get $pixels i64.div_u i64.or))
    (core instance $consume-instance
        (instantiate $consume (with "" (instance
            (export "memory" (memory $libc "memory"))
            (export "stream.read" (func $stream.read))))))
    (func (export "consume") async (param "input" $bytes) (result u64)
        (canon lift (core func $consume-instance "consume")
            (memory (core memory $libc "memory")))))
