// The probe's program (run.sh): built with the generated Swift bindings and linked against ngterm-mobile's iOS static
// library. The first call checks UniFFI's contract version and every checksum; generateKey then runs russh's key
// generation inside the simulator.
print("core \(coreVersion())")
let key = try generateKey(comment: "ios-probe")
print("key \(key.authorizedKey.prefix(11)) \(key.fingerprint.prefix(7))")
