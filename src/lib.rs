use std::{cell::UnsafeCell,  sync::atomic::{
	AtomicU32,
	Ordering
}};


// Trait implementation for the optimistic spin lock state machine, built for small stack based POD
// This lock is built to be fast as possible meaning it does not make any syscalls and doesn't increment the atomic on reads
// Since the lock  does not increment the atomic reference counters for readers, the writers get priority and will starve readers under high writer workloads
// Unlike a normal locks where readers increment the count, instead we use generation where after a writer unlocks the control bit it increments the generation
// Thefore readers must ensure the generation does not change, and that the writer control bit is not flipped before returning a copy of the data to avoid corruption
// This allows concurrent lock free readers without any atomic fetch or adds, just atomic loads on the state + memcpy
// Its only suited well for small structs ideally 128 bytes or less, 64 is optimal
pub trait SpinLockLogic {
	/// [31: writer bit, 30-0: writer generation increment] - if gen increment >=  2^31 then rollover to 0 to avoid writer bit corruption
	///type Control; // Atomic u32
	const U32_MASK: u32 = 1 << 31;

    /// Function for binding a structs AtomicU32 to the trait logic
	fn state(&self) -> &AtomicU32;

	// Helpers 
	/// Returns the current generation increment 
	fn generation(&self) -> u32 {
		self.state().load(Ordering::Acquire) & !Self::U32_MASK
	}

	// Check if the writer bit is set
	fn active_writer(&self) -> bool {
		(self.state().load(Ordering::Acquire) & Self::U32_MASK) != 0
	}


	// Writers
	/// Implements exponential backoff for spin locking followed by a thread yeild
	fn lock(&self) -> u32 {
		let mut shift = 0;
		const MAX_BACKOFF: u32 = 10;
		const SHIFT_RESET: u32 = 4;

		loop {
			// if old state was 0 and we or it to 1 then return, otherwise we flipped 1 to 1 and need to retry
			let old_state = self.state().fetch_or(Self::U32_MASK, Ordering::Acquire);
			if (old_state & Self::U32_MASK) == 0 {
				return old_state;
			} 

			// Exponential backoff, increasing spin loop duration up to a predefined upper bound
			if shift < MAX_BACKOFF {
				let spins: u32 = 1 << shift;
				for _ in 0..spins {
					std::hint::spin_loop();
				}
				shift += 1;
			} else {
				// Calls yeild to reduce thread utilization & contention issues
				std::thread::yield_now();
				shift = SHIFT_RESET;
			}
		}
	}

	/// Unlocks the writer control bit and increments the generation
	fn unlock(&self) {
		let cur = self.state().load(Ordering::Relaxed);

		// single clear
		let next_gen = (cur.wrapping_add(1)) & !Self::U32_MASK;
		// double clear
		//let next = (cur & !Self::U32_MASK).wrapping_add(1) & !Self::U32_MASK;
		self.state().store(next_gen, Ordering::Release);
	}
}

// --------------------

/// Spin Lock header Struct
pub struct SpinLock<T: Copy> {
	control: AtomicU32,
	data: UnsafeCell<T>
}

impl<T: Copy> SpinLock<T> {
	pub fn new(val: T) -> Self {
		Self {
			control: AtomicU32::new(0),
			data: UnsafeCell::new(val)
		}
	}

	/// Uses no backoff to maximize read cycles since all we call are atomic loads + mempcpy no atomic updates like fetch_or
	pub fn read(&self) -> T {
		loop {
			// get current generation - baseline
			let before = self.state().load(Ordering::Acquire);
			
			// If a writer is active then spin
			if (before & <Self as SpinLockLogic>::U32_MASK) != 0 {
				std::hint::spin_loop();
				continue;
			}

			// Potentially copying a currupted state here
			let val = unsafe { std::ptr::read(self.data.get()) };

			
			// if the payload has not changed and the writer bit is 0 then return it
			if self.state().load(Ordering::Acquire) == before {
				return val;
			}

			// spin before retry loop
			std::hint::spin_loop();
		}
	}

	pub fn lock(&self) -> SpinLockGuard<'_, T> {
		<Self as SpinLockLogic>::lock(self);
		SpinLockGuard { lock: self }
	}
}


unsafe impl<T: Copy + Send> Send for SpinLock<T> {}
unsafe impl<T: Copy + Send> Sync for SpinLock<T> {}

impl<T: Copy> SpinLockLogic for SpinLock<T> {
	fn state(&self) -> &AtomicU32 {
		&self.control
	}
}


// --------------------


/// Spin lock header guard
pub struct SpinLockGuard<'a, T: Copy> {
	lock: &'a SpinLock<T>
}

impl<T: Copy> std::ops::Deref for SpinLockGuard<'_, T> {
	type Target = T;
	fn deref(&self) -> &Self::Target {
		unsafe { &*self.lock.data.get() }
	} 
}

impl<T: Copy> std::ops::DerefMut for SpinLockGuard<'_, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		unsafe { &mut *self.lock.data.get() }
	}
}

/// When the guard gets descoped or dropped it calls unlock automatically
impl<T: Copy> Drop for SpinLockGuard<'_, T> {
	fn drop(&mut self) {
		self.lock.unlock();
	}
}

