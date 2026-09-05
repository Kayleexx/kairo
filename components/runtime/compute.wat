(component
    (core module $compute
        (func (export "compute") (param $input i32) (result i32)
            (local $number i32)
            (local $divisor i32)
            (local $count i32)
            (local $is-prime i32)
            i32.const 2
            local.set $number
            (block $done
                (loop $numbers
                    local.get $number
                    local.get $input
                    i32.gt_u
                    br_if $done
                    i32.const 1
                    local.set $is-prime
                    i32.const 2
                    local.set $divisor
                    (block $checked
                        (loop $divisors
                            local.get $divisor
                            local.get $divisor
                            i32.mul
                            local.get $number
                            i32.gt_u
                            br_if $checked
                            local.get $number
                            local.get $divisor
                            i32.rem_u
                            i32.eqz
                            if
                                i32.const 0
                                local.set $is-prime
                                br $checked
                            end
                            local.get $divisor
                            i32.const 1
                            i32.add
                            local.set $divisor
                            br $divisors))
                    local.get $is-prime
                    if
                        local.get $count
                        i32.const 1
                        i32.add
                        local.set $count
                    end
                    local.get $number
                    i32.const 1
                    i32.add
                    local.set $number
                    br $numbers))
            local.get $count))
    (core instance $compute-instance (instantiate $compute))
    (type $compute-type (func async (param "input" u32) (result u32)))
    (func $compute (type $compute-type)
        (canon lift (core func $compute-instance "compute")))
    (export "compute" (func $compute)))
