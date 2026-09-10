(component
    (core module $libc (memory (export "memory") 3))
    (core instance $libc (instantiate $libc))
    (type $bytes (stream u8))
    (core func $stream.read
        (canon stream.read $bytes async (memory (core memory $libc "memory"))))
    (core module $consume
        (import "" "memory" (memory 3))
        (import "" "stream.read" (func $read (param i32 i32 i32) (result i32)))
        (data (i32.const 131072) "id")
        (data (i32.const 131074) "amount_cents")
        (data (i32.const 131086) "status")
        (data (i32.const 131092) "id,amount_cents,status")
        (global $position (mut i32) (i32.const 0))
        (global $length (mut i32) (i32.const 0))
        (global $mode (mut i32) (i32.const 0))
        (global $records (mut i64) (i64.const 0))
        (global $total (mut i64) (i64.const 0))

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

        (func $space
            (block $done
                (loop $next
                    global.get $position global.get $length i32.ge_u br_if $done
                    global.get $position i32.load8_u
                    i32.const 32 i32.eq
                    global.get $position i32.load8_u i32.const 9 i32.eq i32.or
                    i32.eqz br_if $done
                    global.get $position i32.const 1 i32.add global.set $position
                    br $next)))

        (func $take (param $byte i32)
            global.get $position global.get $length i32.ge_u if unreachable end
            global.get $position i32.load8_u local.get $byte i32.ne if unreachable end
            global.get $position i32.const 1 i32.add global.set $position)

        (func $hex (param $byte i32) (result i32)
            local.get $byte i32.const 48 i32.ge_u
            local.get $byte i32.const 57 i32.le_u i32.and
            local.get $byte i32.const 65 i32.ge_u
            local.get $byte i32.const 70 i32.le_u i32.and i32.or
            local.get $byte i32.const 97 i32.ge_u
            local.get $byte i32.const 102 i32.le_u i32.and i32.or)

        (func $json-string (result i32)
            (local $count i32) (local $byte i32) (local $escaped i32)
            i32.const 34 call $take
            (block $done
                (loop $next
                    global.get $position global.get $length i32.ge_u if unreachable end
                    global.get $position i32.load8_u local.set $byte
                    global.get $position i32.const 1 i32.add global.set $position
                    local.get $byte i32.const 34 i32.eq br_if $done
                    local.get $byte i32.const 32 i32.lt_u if unreachable end
                    local.get $byte i32.const 92 i32.eq
                    if
                        global.get $position global.get $length i32.ge_u if unreachable end
                        global.get $position i32.load8_u local.set $escaped
                        global.get $position i32.const 1 i32.add global.set $position
                        local.get $escaped i32.const 117 i32.eq
                        if
                            i32.const 0 local.set $byte
                            (block $hex-done
                                (loop $hex-next
                                    local.get $byte i32.const 4 i32.ge_u br_if $hex-done
                                    global.get $position global.get $length i32.ge_u if unreachable end
                                    global.get $position i32.load8_u call $hex i32.eqz if unreachable end
                                    global.get $position i32.const 1 i32.add global.set $position
                                    local.get $byte i32.const 1 i32.add local.set $byte
                                    br $hex-next))
                        else
                            local.get $escaped i32.const 34 i32.eq
                            local.get $escaped i32.const 92 i32.eq i32.or
                            local.get $escaped i32.const 47 i32.eq i32.or
                            local.get $escaped i32.const 98 i32.eq i32.or
                            local.get $escaped i32.const 102 i32.eq i32.or
                            local.get $escaped i32.const 110 i32.eq i32.or
                            local.get $escaped i32.const 114 i32.eq i32.or
                            local.get $escaped i32.const 116 i32.eq i32.or
                            i32.eqz if unreachable end
                        end
                    end
                    local.get $count i32.const 1 i32.add local.set $count
                    br $next))
            local.get $count)

        (func $json-key (result i32)
            (local $start i32) (local $key-length i32) (local $byte i32)
            i32.const 34 call $take
            global.get $position local.set $start
            (block $done
                (loop $next
                    global.get $position global.get $length i32.ge_u if unreachable end
                    global.get $position i32.load8_u local.set $byte
                    local.get $byte
                    i32.const 34 i32.eq br_if $done
                    local.get $byte i32.const 32 i32.lt_u
                    local.get $byte i32.const 92 i32.eq i32.or if unreachable end
                    global.get $position i32.const 1 i32.add global.set $position
                    br $next))
            global.get $position local.get $start i32.sub local.set $key-length
            global.get $position i32.const 1 i32.add global.set $position
            local.get $start local.get $key-length i32.const 131072 i32.const 2 call $equal
            if i32.const 1 return end
            local.get $start local.get $key-length i32.const 131074 i32.const 12 call $equal
            if i32.const 2 return end
            local.get $start local.get $key-length i32.const 131086 i32.const 6 call $equal
            if i32.const 3 return end
            unreachable)

        (func $unsigned (result i64)
            (local $value i64) (local $byte i32) (local $digits i32)
            (block $done
                (loop $next
                    global.get $position global.get $length i32.ge_u br_if $done
                    global.get $position i32.load8_u local.set $byte
                    local.get $byte
                    i32.const 48 i32.lt_u br_if $done
                    local.get $byte i32.const 57 i32.gt_u br_if $done
                    local.get $value i64.const 429496729 i64.gt_u if unreachable end
                    local.get $value i64.const 10 i64.mul
                    local.get $byte i32.const 48 i32.sub i64.extend_i32_u i64.add local.set $value
                    local.get $value i64.const 4294967295 i64.gt_u if unreachable end
                    global.get $position i32.const 1 i32.add global.set $position
                    local.get $digits i32.const 1 i32.add local.set $digits
                    br $next))
            local.get $digits i32.eqz if unreachable end
            local.get $value)

        (func $json-record (result i64)
            (local $mask i32) (local $key i32) (local $amount i64)
            i32.const 0 global.set $position
            call $space i32.const 123 call $take call $space
            (block $done
                (loop $field
                    global.get $position i32.load8_u i32.const 125 i32.eq br_if $done
                    call $json-key local.set $key
                    i32.const 1 local.get $key i32.const 1 i32.sub i32.shl
                    local.get $mask i32.and if unreachable end
                    local.get $mask i32.const 1 local.get $key i32.const 1 i32.sub i32.shl i32.or
                    local.set $mask
                    call $space i32.const 58 call $take call $space
                    local.get $key i32.const 2 i32.eq
                    if call $unsigned local.set $amount
                    else call $json-string i32.eqz if unreachable end
                    end
                    call $space
                    global.get $position i32.load8_u i32.const 44 i32.eq
                    if
                        global.get $position i32.const 1 i32.add global.set $position
                        call $space br $field
                    end
                    br $done))
            i32.const 125 call $take call $space
            global.get $position global.get $length i32.ne if unreachable end
            local.get $mask i32.const 7 i32.ne if unreachable end
            local.get $amount)

        (func $csv-field (param $field i32) (result i64)
            (local $quoted i32) (local $count i32) (local $byte i32) (local $value i64)
            global.get $position global.get $length i32.lt_u
            if
                global.get $position i32.load8_u i32.const 34 i32.eq
                if
                    i32.const 1 local.set $quoted
                    global.get $position i32.const 1 i32.add global.set $position
                end
            end
            (block $done
                (loop $next
                    global.get $position global.get $length i32.ge_u br_if $done
                    global.get $position i32.load8_u local.set $byte
                    local.get $quoted
                    if
                        local.get $byte i32.const 34 i32.eq
                        if
                            global.get $position i32.const 1 i32.add global.set $position
                            global.get $position global.get $length i32.lt_u
                            if
                                global.get $position i32.load8_u i32.const 34 i32.eq
                                if i32.const 34 local.set $byte
                                else br $done
                                end
                            else br $done
                            end
                        end
                    else
                        local.get $byte i32.const 44 i32.eq br_if $done
                        local.get $byte i32.const 34 i32.eq if unreachable end
                    end
                    local.get $byte i32.const 32 i32.lt_u if unreachable end
                    local.get $field i32.const 1 i32.eq
                    if
                        local.get $byte i32.const 48 i32.lt_u if unreachable end
                        local.get $byte i32.const 57 i32.gt_u if unreachable end
                        local.get $value i64.const 429496729 i64.gt_u if unreachable end
                        local.get $value i64.const 10 i64.mul
                        local.get $byte i32.const 48 i32.sub i64.extend_i32_u i64.add local.set $value
                        local.get $value i64.const 4294967295 i64.gt_u if unreachable end
                    end
                    local.get $count i32.const 1 i32.add local.set $count
                    global.get $position i32.const 1 i32.add global.set $position
                    br $next))
            local.get $quoted
            if
                global.get $position i32.const 1 i32.sub i32.load8_u i32.const 34 i32.ne
                if unreachable end
            end
            local.get $count i32.eqz if unreachable end
            local.get $value)

        (func $csv-record (result i64)
            (local $field i32) (local $amount i64) (local $value i64)
            i32.const 0 global.set $position
            (loop $next
                local.get $field call $csv-field local.set $value
                local.get $field i32.const 1 i32.eq
                if local.get $value local.set $amount end
                local.get $field i32.const 2 i32.lt_u
                if
                    global.get $position global.get $length i32.ge_u if unreachable end
                    i32.const 44 call $take
                    local.get $field i32.const 1 i32.add local.set $field
                    br $next
                end)
            global.get $position global.get $length i32.ne if unreachable end
            local.get $amount)

        (func $record (param $record-length i32)
            (local $amount i64)
            local.get $record-length global.set $length
            global.get $length i32.const 1 i32.ge_u
            if
                global.get $length i32.const 1 i32.sub i32.load8_u i32.const 13 i32.eq
                if global.get $length i32.const 1 i32.sub global.set $length end
            end
            global.get $length i32.eqz if return end
            global.get $mode i32.eqz
            if
                i32.const 0 global.set $position call $space
                global.get $position global.get $length i32.ge_u if return end
                global.get $position i32.load8_u i32.const 123 i32.eq
                if
                    i32.const 1 global.set $mode
                else
                    i32.const 0 global.set $position
                    i32.const 0 global.get $length i32.const 131092 i32.const 22 call $equal
                    i32.eqz if unreachable end
                    i32.const 2 global.set $mode
                    return
                end
            end
            global.get $mode i32.const 1 i32.eq
            if call $json-record local.set $amount
            else call $csv-record local.set $amount end
            global.get $total local.get $amount i64.add global.set $total
            global.get $total i64.const 4294967295 i64.gt_u if unreachable end
            global.get $records i64.const 1 i64.add global.set $records
            global.get $records i64.const 4294967295 i64.gt_u if unreachable end)

        (func (export "consume") (param $stream i32) (result i64)
            (local $status i32) (local $index i32) (local $byte i32)
            (local $record-length i32)
            (block $done
                (loop $read-more
                    local.get $stream i32.const 65536 i32.const 65536 call $read local.set $status
                    local.get $status i32.const -1 i32.eq if unreachable end
                    i32.const 0 local.set $index
                    (block $chunk-done
                        (loop $bytes
                            local.get $index local.get $status i32.const 4 i32.shr_u i32.ge_u
                            br_if $chunk-done
                            local.get $index i32.const 65536 i32.add i32.load8_u local.set $byte
                            local.get $byte i32.const 10 i32.eq
                            if
                                local.get $record-length call $record
                                i32.const 0 local.set $record-length
                            else
                                local.get $record-length i32.const 65535 i32.ge_u if unreachable end
                                local.get $record-length local.get $byte i32.store8
                                local.get $record-length i32.const 1 i32.add local.set $record-length
                            end
                            local.get $index i32.const 1 i32.add local.set $index
                            br $bytes))
                    local.get $status i32.const 1 i32.and br_if $done
                    br $read-more))
            local.get $record-length call $record
            global.get $mode i32.eqz if unreachable end
            global.get $records i64.eqz if unreachable end
            global.get $records i64.const 32 i64.shl global.get $total i64.or))
    (core instance $consume-instance
        (instantiate $consume (with "" (instance
            (export "memory" (memory $libc "memory"))
            (export "stream.read" (func $stream.read))))))
    (func (export "consume") async (param "input" $bytes) (result u64)
        (canon lift (core func $consume-instance "consume")
            (memory (core memory $libc "memory")))))
