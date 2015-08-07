// Tifflin OS - simple_console
// - By John Hodge (thePowersGang)
//
// Simplistic console, used as a quick test case (fullscreen window)
#![feature(core_slice_ext,core_str_ext)]
#![feature(const_fn)]

#[macro_use]
extern crate syscalls;

extern crate cmdline_words_parser;

use syscalls::Object;

use syscalls::gui::Colour;

mod terminal_surface;
mod terminal;

mod input;

use std::fmt::Write;

fn main() {
	use syscalls::gui::{Group,Window};
	use syscalls::threads::S_THIS_PROCESS;
	
	::syscalls::threads::wait(&mut [S_THIS_PROCESS.get_wait()], !0);
	::syscalls::gui::set_group( S_THIS_PROCESS.receive_object::<Group>(0).unwrap() );

	// Create maximised window
	let window = Window::new("Console").unwrap();
	window.maximise();
	window.fill_rect(0,0, !0,!0, 0x33_00_00);   // A nice rust-like red :)

	// Create terminal
	let mut term = terminal::Terminal::new(&window, ::syscalls::gui::Rect::new(0,0, 1920,1080));
	let mut input = input::InputStack::new();
	// Print header
	{
		let mut buf = [0; 128];
		term.set_foreground( Colour::def_green() );
		let _ = write!(&mut term, "{}\n",  ::syscalls::get_text_info(0, 0, &mut buf));	// Kernel 0: Version line
		term.set_foreground( Colour::def_yellow() );
		let _ = write!(&mut term, " {}\n", ::syscalls::get_text_info(0, 1, &mut buf));	// Kernel 1: Build line
		term.set_foreground( Colour::white() );
		let _ = write!(&mut term, "Simple console\n");
	}
	window.show();
	
	// Initial prompt
	term.write_str("> ").unwrap();
	term.flush();

	let mut shell = ShellState::new();

	loop {
		// Bind to receive events relating to the window
		let mut events = [window.get_wait()];
		
		::syscalls::threads::wait(&mut events, !0);
	
		while let Some(ev) = window.pop_event()
		{
			kernel_log!("ev = {:?}", ev);
			match ev
			{
			::syscalls::gui::Event::KeyUp(kc) => {
				if let Some(buf) = input.handle_key(true, kc as u8, |a| render_input(&mut term, a))
				{
					kernel_log!("buf = {:?}", buf);
					term.write_str("\n").unwrap();
					term.flush();
					window.redraw();

					shell.handle_command(&mut term, buf);
					// - If the command didn't print a newline, print one for it
					if term.cur_col() != 0 {
						term.write_str("\n").unwrap();
					}
					// New prompt
					term.write_str("> ").unwrap();
				}
				term.flush();
				window.redraw();
				},
			::syscalls::gui::Event::KeyDown(kc) => {
				input.handle_key(false, kc as u8, |_| ());
				},
			_ => {},
			}
		}
		
		window.check_wait(&events[0]);
	}
}

// Render callback for input stack
fn render_input(term: &mut terminal::Terminal, action: input::Action)
{
	use input::Action;
	match action
	{
	Action::Backspace => term.delete_left(),
	Action::Delete => term.delete_right(),
	Action::Puts(s) => term.write_str(s).unwrap(),
	}
}

#[derive(Default)]
struct ShellState
{
	/// Current working directory, relative to /
	cwd_rel: String,
}


macro_rules! print {
	($term:expr, $($t:tt)*) => ({use std::fmt::Write; let _ = write!($term, $($t)*);});
}

impl ShellState
{
	pub fn new() -> ShellState {
		Default::default()
	}
	/// Handle a command
	pub fn handle_command(&mut self, term: &mut terminal::Terminal, mut cmdline: String)
	{
		use cmdline_words_parser::StrExt;
		let mut args = cmdline.parse_cmdline_words();
		match args.next()
		{
		None => {},
		// 'pwd' - Print working directory
		Some("pwd") => print!(term, "/{}", self.cwd_rel),
		// 'cd' - Change directory
		Some("cd") =>
			if let Some(p) = args.next()
			{
				print!(term, "TODO: cd '{}'", p);
			}
			else
			{
				self.cwd_rel = String::new();
			},
		// 'ls' - Print the contents of a directory
		Some("ls") =>
			if let Some(dir) = args.next()
			{
				// TODO: Parse 'dir' as relative correctly
				command_ls(term, dir);
			}
			else
			{
				command_ls(term, &format!("/{}", self.cwd_rel));
			},
		// 'cat' - Dump the contents of a file
		// TODO: Implement
		Some("cat") => print!(term, "TODO: cat"),
		// 'echo' - Prints all arguments space-separated
		Some("echo") =>
			while let Some(v) = args.next() {
				print!(term, "{} ", v);
			},
		Some("help") => {
			print!(term, "Builtins: pwd, cd, ls, cat, help, echo");
			},
		Some(cmd @_) => {
			print!(term, "Unkown command '{}'", cmd);
			},
		}
	}
}

fn command_ls(term: &mut terminal::Terminal, path: &str) {
	let mut handle = match ::syscalls::vfs::Dir::open(path)
		{
		Ok(v) => v,
		Err(e) => {
			print!(term, "Unable to open '{}': {:?}", path, e);
			return ;
			},
		};
	
	let mut buf = [0; 256];
	loop
	{
		let name_bytes = match handle.read_ent(&mut buf)
			{
			Ok(v) => v,
			Err(e) => {
				print!(term, "Read error: {:?}", e);
				return ;
				},
			};

		let name = ::std::str::from_utf8(name_bytes);
		print!(term, "- {:?}\n", name);
		if name_bytes == b"" { break ; }
	}
}
