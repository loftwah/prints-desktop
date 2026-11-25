#![forbid(unsafe_code)]

use std::io::Write;
use std::path::Path;

use prints_desktop::oyster::{ApiKey, OysterClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (origin, installation, export) = match args.as_slice() {
        [origin, installation] => (origin, installation, None),
        [origin, installation, option, print, path] if option == "--export" => {
            (origin, installation, Some((print, Path::new(path))))
        }
        _ => {
            eprintln!(
                "Usage: oyster-reader <origin> <installation> [--export <print-id> <new-file>]"
            );
            std::process::exit(2);
        }
    };
    let key = rpassword::prompt_password("Scoped Prints App key (hidden): ")?;
    let client = OysterClient::new(origin, installation, ApiKey::new(key))?;
    let page = client.list_outputs(None, true, None, 24)?;
    let scope = if client.root_only() {
        "the root library"
    } else {
        "the library"
    };
    println!("Read {} entries from {scope}.", page.items.len());
    match client.list_models() {
        Ok(catalogue) => println!("Read {} model entries.", catalogue.models.len()),
        Err(error) => eprintln!("Model catalogue unavailable: {error}"),
    }
    if let Some((print_id, path)) = export {
        let original = client.download_output(print_id)?;
        let mut file = std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(path)?;
        let written = file.write_all(&original).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = written {
            let _ = std::fs::remove_file(path);
            return Err(error.into());
        }
        println!("Exported {} original bytes.", original.len());
    }
    Ok(())
}
