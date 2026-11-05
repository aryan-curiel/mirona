//! `mirona` CLI (NGR-149). Reads the same store as the app.
//! Usage: mirona meetings | mirona show <id> | mirona actions | mirona search <query>

use mirona_store::Store;

fn main() {
    let store = Store::sample();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = args.get(1).map_or("", String::as_str);
    match args.first().map(String::as_str) {
        Some("meetings") => {
            for m in store.list_meetings() {
                println!("{:<18} {:<28} {} · {}", m.id, m.title, m.client, m.when);
            }
        }
        Some("show") => match store.meeting(arg) {
            Ok(m) => {
                println!("# {}\n{} · {} min\n", m.title, m.date_label, m.duration_min);
                for s in &m.segments {
                    let who = m
                        .speakers
                        .iter()
                        .find(|p| p.id == s.speaker_id)
                        .map_or("?", |p| &p.name);
                    let text = if s.redacted { "[redacted]" } else { &s.text };
                    println!(
                        "{:02}:{:02} {who}: {text}",
                        s.start_ms / 60_000,
                        s.start_ms / 1000 % 60
                    );
                }
            }
            Err(e) => eprintln!("{e}"),
        },
        Some("actions") => {
            for a in &store.action_items() {
                let done = if a.done { "x" } else { " " };
                println!(
                    "[{done}] {} · {} · {}",
                    a.text,
                    a.owner,
                    a.due.as_deref().unwrap_or("-")
                );
            }
        }
        Some("search") => {
            for m in store.search(arg) {
                println!("{} · {}", m.id, m.title);
            }
        }
        _ => eprintln!("usage: mirona meetings | show <id> | actions | search <query>"),
    }
}
