//! Diagnostic: dump every key of every row in a raw preset-list stream (`fretwire dump-list`),
//! to see whether the listing carries anything beyond the name — a populated flag in particular.
use fretwire_data::rmpv::Value;
use fretwire_data::stream::{locate_root_where, map_get};

fn main() {
    let path = std::env::args().nth(1).expect("usage: list_rows <file>");
    let bytes = std::fs::read(&path).unwrap();
    let root = locate_root_where(&bytes, 32, |v| {
        matches!(map_get(v, 104), Some(Value::Array(_)))
    })
    .expect("no listing envelope");
    let Some(Value::Array(rows)) = map_get(&root.value, 104) else {
        unreachable!()
    };
    println!("{} rows", rows.len());
    for (pos, row) in rows.iter().enumerate() {
        let Value::Map(m) = row else {
            println!("[{pos}] not a map: {row}");
            continue;
        };
        for (k, inner) in m {
            match inner {
                Value::Map(fields) => {
                    let rendered: Vec<String> =
                        fields.iter().map(|(fk, fv)| format!("{fk}={fv}")).collect();
                    println!("[{pos:3}] key={k} {{{}}}", rendered.join(", "));
                }
                other => println!("[{pos:3}] key={k} {other}"),
            }
        }
    }
}
