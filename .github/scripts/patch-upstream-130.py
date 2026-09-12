from pathlib import Path

p = Path("src/vector/storage.rs")
s = p.read_text()
s = s.replace(
    "use std::io::{self, Write};",
    "use std::io::{self, BufWriter, Write};",
    1,
)
old = """        // Write vectors
        for (id, vector) in vectors {
            // Write vector ID
            file.write_all(&id.to_bytes())?;

            // Write vector data
            for &value in *vector {
                file.write_all(&value.to_le_bytes())?;
            }
        }

        file.flush()?;
"""
new = """        // Buffer payload writes. A 384-dimensional vector otherwise turns
        // into 385 small write_all calls. Flush before update_metadata() can
        // publish the new header count, preserving the existing crash ordering.
        let mut writer = BufWriter::new(file);
        for (id, vector) in vectors {
            writer.write_all(&id.to_bytes())?;
            for &value in *vector {
                writer.write_all(&value.to_le_bytes())?;
            }
        }
        writer.flush()?;
"""
if old not in s:
    raise SystemExit("current zero-copy payload block not found")
p.write_text(s.replace(old, new, 1))
