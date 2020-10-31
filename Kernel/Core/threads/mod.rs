// "Tifflin" Kernel
// - By John Hodge (thePowersGang)
//
// Core/threads/mod.rs
//! Thread management

mod thread;
mod thread_list;
mod wait_queue;

mod worker_thread;

mod sleep_object;

pub use self::thread::{Thread,ThreadPtr,ThreadID,ProcessID};
pub use self::thread::{ThreadHandle,ProcessHandle};
pub use self::thread::new_idle_thread;

pub use self::worker_thread::WorkerThread;

pub use self::thread_list::{ThreadList,THREADLIST_INIT};
pub use self::sleep_object::{SleepObject,SleepObjectRef};
pub use self::wait_queue::WaitQueue;

use lib::mem::aref::{Aref,ArefBorrow};

/// A bitset of wait events
pub type EventMask = u32;

// ----------------------------------------------
// Statics
//static s_all_threads:	::sync::Mutex<Map<uint,*const Thread>> = mutex_init!(Map{});
#[allow(non_upper_case_globals)]
static s_runnable_threads: ::sync::Spinlock<ThreadList> = ::sync::Spinlock::new(THREADLIST_INIT);
static S_PID0: ::lib::LazyStatic<::lib::mem::Arc<thread::Process>> = ::lib::LazyStatic::new();
// Spinlocked due to low contention, and because the current thread is pushed to it
static S_TO_REAP_THREADS: ::sync::Spinlock<ThreadList> = ::sync::Spinlock::new(THREADLIST_INIT);

// ----------------------------------------------
// Code
/// Initialise the threading subsystem
pub fn init()
{
	// SAFE: Runs before any form of multi-threading starts
	unsafe {
		S_PID0.prep( || thread::Process::new_pid0() )
	}
	let mut tid0 = Thread::new_boxed(0, "ThreadZero", S_PID0.clone());
	tid0.cpu_state = ::arch::threads::init_tid0_state();
	::arch::threads::set_thread_ptr( tid0 );
}

/// Returns `true` if a thread was reaped
fn reap_threads() -> bool
{
	let mut rv = false;
	while let Some(thread) = S_TO_REAP_THREADS.lock().pop() {
		log_log!("Reaping thread {:?}", thread);
		assert!(&*thread as *const Thread != ::arch::threads::borrow_thread() as *const _, "Reaping thread from itself");
		match thread.into_boxed()
		{
		Ok(thread) => drop(thread),
		Err(thread) => log_warning!("Attempting reap 'static thread {:?}", thread),
		}
		rv = true;
	}
	rv
}

pub fn idle_thread()
{
	loop
	{
		if ! reap_threads()
		{
			// SAFE: I know what I'm doing, and we trust idle() to re-enable them
			unsafe { ::arch::sync::stop_interrupts(); }
			if let Some(thread) = get_thread_to_run() {
				// SAFE: We turned them off, we turn them back on
				unsafe { ::arch::sync::start_interrupts(); }
				log_debug!("Idle task switch to {:?}", thread);
				::arch::threads::switch_to(thread);
			}
			else {
				// NOTE: Idle _must_ re-enable interrupts
				::arch::threads::idle();
			}
		}
		else
		{
			reschedule();
		}
	}
}

/// Yield control of the CPU for a short period (while polling or main thread halted)
pub fn yield_time()
{
	// HACK: Drop to-reap threads in this function
	reap_threads();

	// Add current thread to active queue, then reschedule
	s_runnable_threads.lock().push( get_cur_thread() );
	reschedule();
}

pub fn yield_to(thread: ThreadPtr)
{
	log_debug!("Yielding CPU to {:?}", thread);
	s_runnable_threads.lock().push( get_cur_thread() );
	::arch::threads::switch_to( thread );
}

pub fn terminate_thread() -> !
{
	// NOTE: If TID0 (aka init's main thread) terminates, panic the kernel
	if with_cur_thread(|cur| cur.get_tid() == 0) {
		panic!("TID 0 terminated");
	}

	// NOTE: Can this just obtain a handle to the current thread then drop it?
	// - No... kinda needs to be properly reaped. (so that no outstanding pointers exist)
	//
	// Set state to "Dead"
	let mut this_thread = get_cur_thread();
	this_thread.set_state( thread::RunState::Dead(0) );
	S_TO_REAP_THREADS.lock().push( this_thread );
	// Reschedule
	// - The idle thread will handle reaping?
	reschedule();
	unreachable!();
}

pub fn exit_process(status: u32) -> ! {
	// Requirements:
	// - Save exit status somewhere
	match with_cur_thread( |cur| cur.get_process_info().mark_exit(status) )
	{
	Ok(_) => {},
	Err(_) => todo!("Two threads raced to exit"),
	}
	log_notice!("Terminating process with status={:#x}", status);

	// - Request all other threads terminate
	// TODO: How would this be done cleanly? Need to wake all and terminate on syscall boundary?
	
	// - Terminate this thread
	//  > Process reaping is handled by the PCB dropping when refcount reaches zero
	terminate_thread();
}

pub fn get_thread_id() -> thread::ThreadID
{
	let p = ::arch::threads::borrow_thread();
	// SAFE: Checks for NULL, and the thread should be vaild while executing
	unsafe {
		if p == 0 as *const _ {
			0
		}
		else {
			(*p).get_tid()
		}
	}
}
pub fn get_process_id() -> thread::ProcessID {
	// SAFE: Does NULL check. TODO: _could_ cause & alias...
	let p = unsafe {
		let p = ::arch::threads::borrow_thread();
		assert!(p != 0 as *const _);
		&*p
		};
	p.get_process_info().get_pid()
}

fn with_cur_thread<T, F: FnOnce(&thread::Thread)->T>(fcn: F) -> T
{
	// SAFE: Checks for NULL, and the thread should be vaild while executing
	let t = unsafe {
		let tp = ::arch::threads::borrow_thread();
		assert!( !tp.is_null() );
		&*tp
		};
	fcn(t)
}

// TODO: Prevent this pointer from being sent (which will prevent accessing of freed memory)
pub fn get_process_local<T: Send+Sync+::core::any::Any+Default+'static>() -> ArefBorrow<T>
{
	// SAFE: Checks for NULL, and the thread should be vaild while executing
	let t = unsafe {
		let tp = ::arch::threads::borrow_thread();
		assert!( !tp.is_null() );
		&*tp
		};
	
	let pld = &t.get_process_info().proc_local_data;
	// 1. Try without write-locking
	for s in pld.read().iter()
	{
		let item_ref: &dyn core::any::Any = &**s;
		//log_debug!("{:?} ?== {:?}", item_ref.type_id(), ::core::any::TypeId::of::<T>());
		if item_ref.type_id() == ::core::any::TypeId::of::<T>() {
			return s.borrow().downcast::<T>().ok().unwrap();
		}
	}
	
	// 2. Try _with_ write locking
	let mut lh = pld.write();
	for s in lh.iter() {
		let item_ref: &dyn core::any::Any = &**s;
		//log_debug!("{:?} ?== {:?}", item_ref.type_id(), ::core::any::TypeId::of::<T>());
		if item_ref.type_id() == ::core::any::TypeId::of::<T>() {
			return s.borrow().downcast::<T>().ok().unwrap();
		}
	}
	// 3. Create an instance
	log_debug!("Creating instance of {} for {}", type_name!(T), t.get_process_info());
	let buf = Aref::new(T::default());
	let ret = buf.borrow();
	lh.push( buf );
	ret
}

/// Pick a new thread to run and run it
///
/// NOTE: This can lead to the current thread being forgotten
#[doc(hidden)]
pub fn reschedule()
{
	loop
	{
		if let Some(thread) = get_thread_to_run()
		{
			if &*thread as *const _ == ::arch::threads::borrow_thread() as *const _
			{
				log_debug!("Task switch to self, idle");
				::arch::threads::switch_to(thread);
				::arch::threads::idle();
			}
			else
			{
				log_debug!("Task switch to {:?}", thread);
				::arch::threads::switch_to(thread);
				//log_debug!("Awoke");
			}
			return ;
		}
		else
		{
			let thread = ::arch::threads::get_idle_thread();
			if &*thread as *const _ != ::arch::threads::borrow_thread() as *const _
			{
				log_trace!("reschedule() - No active threads, idling");
				
				// Switch to the idle thread
				::arch::threads::switch_to( thread );
			}
			else {
				::core::mem::forget(thread);
			}
			return ;
		}
	}
}

fn get_cur_thread() -> ThreadPtr
{
	::arch::threads::get_thread_ptr().expect("Current thread is None")
}
fn rel_cur_thread(t: ThreadPtr)
{
	::arch::threads::set_thread_ptr(t)
}
//fn borrow_cur_thread() -> BorrowedThread
//{
//	BorrowedThread( Some(get_cur_thread()) )
//}

fn get_thread_to_run() -> Option<ThreadPtr>
{
	let _irq_lock = ::arch::sync::hold_interrupts();
	let mut handle = s_runnable_threads.lock();
	if handle.empty()
	{
		// WTF? At least an idle thread should be ready
		None
	}
	else
	{
		// 2. Pop off a new thread
		handle.pop()
	}
}

// vim: ft=rust

