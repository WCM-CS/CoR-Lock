// Trait implementation for the optimistic spin lock state machine, built for small stack based POD
// This lock is built to be fast as possible meaning it does not make any syscalls and doesn't increment the atomic on reads
// Since the lock  does not increment the atomic reference counters for readers, the writers get priority and will starve readers under high writer workloads
// Unlike a normal locks where readers increment the count, instead we use generation where after a writer unlocks the control bit it increments the generation
// Thefore readers must ensure the generation does not change, and that the writer control bit is not flipped before returning a copy of the data to avoid corruption
// This allows concurrent lock free readers without any atomic fetch or adds, just atomic loads on the state + memcpy
// Its only suited well for small structs ideally 128 bytes or less, 64 is optimal
