use anyhow::{bail, Context, Result};
use clap::Parser;
use csv::ReaderBuilder;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

// Hilfsfunktionen für die Konvertierung
fn convert_date(date_str: &str) -> Result<String> {
    let date_parts: Vec<&str> = date_str.split('/').collect();
    if date_parts.len() != 3 {
        bail!("Ungültiges Datumsformat: {}", date_str);
    }
    Ok(format!(
        "20{}{}{}",
        date_parts[2], date_parts[1], date_parts[0]
    ))
}

fn convert_price(price_whole: &str, price_decimal: &str) -> Result<String> {
    let whole: i32 = price_whole
        .parse()
        .with_context(|| format!("Ungültiger Preis: {}", price_whole))?;
    let decimal: i32 = price_decimal
        .parse()
        .with_context(|| format!("Ungültige Dezimalstellen: {}", price_decimal))?;
    // Wenn die Dezimalstelle einstellig ist UND der String auch einstellig ist,
    // multipliziere mit 10 (z.B. "6" wird zu "60")
    let decimal = if decimal < 10 && price_decimal.len() == 1 {
        decimal * 10
    } else {
        decimal
    };
    Ok(format!("{}.{:02}", whole, decimal))
}

fn convert_quantity(amount: &str, trade_type: &str) -> Result<i32> {
    let amount: i32 = amount
        .parse()
        .with_context(|| format!("Ungültige Menge: {}", amount))?;
    if !matches!(trade_type, "B" | "S") {
        bail!("Ungültiger Handelstyp: {}", trade_type);
    }
    Ok(if trade_type == "S" { -amount } else { amount })
}

#[cfg(target_os = "windows")]
mod windows {
    use anyhow::Result;
    use std::path::PathBuf;
    use winreg::enums::*;
    use winreg::RegKey;

    pub fn install_context_menu() -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Classes\.csv\shell\Mutate\command";
        let (key, _) = hkcu.create_subkey(path)?;

        let exe_path = std::env::current_exe()?;
        let command = format!("\"{}\" convert \"%1\"", exe_path.display());

        key.set_value("", &command)?;
        Ok(())
    }

    pub fn uninstall_context_menu() -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Classes\.csv\shell\Mutate";
        hkcu.delete_subkey_all(path)?;
        Ok(())
    }
}

#[derive(Parser)]
#[command(name = "Mutate")]
#[command(author = "eddi888")]
#[command(version = "1.1.0")]
#[command(about = "Konvertiert Sterling Trader Exportdateien in das TradesViz Format")]
struct Cli {
    /// Eingabedatei (Sterling Trader Export)
    input: String,

    /// Ausgabedatei (Optional, Standard: <input>.txt.cust.csv)
    #[arg(short, long)]
    output: Option<String>,

    /// Zeige detaillierte Informationen während der Konvertierung
    #[arg(short, long)]
    verbose: bool,

    #[cfg(target_os = "windows")]
    /// Installiere Windows-Kontextmenü Integration
    #[arg(long)]
    install: bool,

    #[cfg(target_os = "windows")]
    /// Deinstalliere Windows-Kontextmenü Integration
    #[arg(long)]
    uninstall: bool,
}

fn convert_file(input_path: &Path, output_path: &Path, verbose: bool) -> Result<()> {
    // Öffne und lese die Eingabedatei
    let mut file = File::open(input_path)
        .with_context(|| format!("Konnte Datei nicht öffnen: {}", input_path.display()))?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| "Fehler beim Lesen der Datei")?;

    // Entferne Nullbytes und BOM
    let mut filtered_bytes: Vec<u8> = bytes.into_iter().filter(|&b| b != 0).collect();
    if filtered_bytes.starts_with(&[0xFF, 0xFE]) || filtered_bytes.starts_with(&[0xFE, 0xFF]) {
        filtered_bytes = filtered_bytes[2..].to_vec();
    }

    // Konfiguriere CSV Reader und Writer
    let cursor = Cursor::new(filtered_bytes);
    let mut reader = ReaderBuilder::new()
        .delimiter(b',')
        .has_headers(false)
        .flexible(true)
        .from_reader(cursor);

    let mut writer = csv::Writer::from_path(output_path).with_context(|| {
        format!(
            "Konnte Ausgabedatei nicht erstellen: {}",
            output_path.display()
        )
    })?;

    // Schreibe Header
    writer.write_record([
        "date",
        "time",
        "symbol",
        "asset_type",
        "price",
        "currency",
        "quantity",
        "commission",
        "tags",
        "notes",
    ])?;

    let mut count = 0;
    for result in reader.records() {
        let record =
            result.with_context(|| format!("Fehler beim Lesen von Zeile {}", count + 1))?;

        // Überprüfe Mindestanzahl der Spalten (mindestens 6 für Zeilen ohne Dezimalstellen)
        if record.len() < 6 {
            bail!(
                "Ungültiges Format in Zeile {}: Zu wenige Spalten (mindestens 6 benötigt)",
                count + 1
            );
        }

        // Konvertiere Datum
        let datum = convert_date(&record[0])
            .with_context(|| format!("Ungültiges Datumsformat in Zeile {}", count + 1))?;

        // Bestimme Position des Handelstyps (B/S)
        let (price_decimal, trade_type) = if record[5].trim() == "B" || record[5].trim() == "S" {
            // Fall ohne Dezimalstellen
            ("0", &record[5])
        } else {
            // Normaler Fall mit Dezimalstellen
            (&record[5], &record[6])
        };

        // Konvertiere Preis
        let preis = convert_price(&record[4], price_decimal)
            .with_context(|| format!("Ungültiger Preis in Zeile {}", count + 1))?;

        // Konvertiere Menge
        let quantity = convert_quantity(&record[3], trade_type.trim())
            .with_context(|| format!("Ungültige Menge in Zeile {}", count + 1))?;

        // Schreibe Datensatz
        writer.write_record([
            &datum,
            &record[1],
            &record[2],
            "stock",
            &preis.to_string(),
            "USD",
            &quantity.to_string(),
            "0.0",
            "",
            "",
        ])?;

        if verbose {
            println!(
                "Verarbeite: {} {} {} @ {}",
                datum, &record[2], quantity, preis
            );
        }

        count += 1;
    }

    writer.flush()?;
    println!(
        "Konvertierung abgeschlossen. {} Datensätze verarbeitet.",
        count
    );
    println!("Ausgabe in: {}", output_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_convert_date() {
        assert_eq!(convert_date("01/04/25").unwrap(), "20250401");
        assert_eq!(convert_date("31/12/24").unwrap(), "20241231");
        assert!(convert_date("invalid").is_err());
        assert!(convert_date("01-04-25").is_err());
    }

    #[test]
    fn test_convert_price() {
        assert_eq!(convert_price("2", "34").unwrap(), "2.34");
        assert_eq!(convert_price("0", "05").unwrap(), "0.05");
        assert_eq!(convert_price("10", "00").unwrap(), "10.00");
        // Test für einstellige Dezimalstellen (z.B. 6 bedeutet 60 Cents)
        assert_eq!(convert_price("1", "6").unwrap(), "1.60");
        assert_eq!(convert_price("2", "5").unwrap(), "2.50");
        assert!(convert_price("invalid", "34").is_err());
        assert!(convert_price("2", "invalid").is_err());
    }

    #[test]
    fn test_convert_quantity() {
        assert_eq!(convert_quantity("1000", "B").unwrap(), 1000);
        assert_eq!(convert_quantity("1000", "S").unwrap(), -1000);
        assert!(convert_quantity("invalid", "B").is_err());
        assert!(convert_quantity("1000", "X").is_err());
    }

    #[test]
    fn test_full_conversion() -> Result<()> {
        // Erstelle eine temporäre Eingabedatei
        let mut input_file = NamedTempFile::new()?;
        // Test für normale Zeilen (mit Dezimalstellen)
        writeln!(input_file, "01/04/25,09:30:00,AAPL,1000,150,50,B")?;
        writeln!(input_file, "01/04/25,09:35:00,AAPL,500,151,25,S")?;
        // Test für Zeilen ohne Dezimalstellen
        writeln!(input_file, "01/04/25,09:40:00,MSFT,1000,160,B")?;
        writeln!(input_file, "01/04/25,09:45:00,MSFT,500,161,S")?;

        // Erstelle eine temporäre Ausgabedatei
        let output_file = NamedTempFile::new()?;

        // Konvertiere die Datei
        convert_file(
            Path::new(input_file.path()),
            Path::new(output_file.path()),
            false,
        )?;

        // Lese die Ausgabedatei
        let output_content = fs::read_to_string(output_file.path())?;
        let output_lines: Vec<&str> = output_content.lines().collect();

        // Überprüfe Header
        assert_eq!(
            output_lines[0],
            "date,time,symbol,asset_type,price,currency,quantity,commission,tags,notes"
        );

        // Überprüfe erste Zeile (Kauf mit Dezimalstellen)
        let fields: Vec<&str> = output_lines[1].split(',').collect();
        assert_eq!(fields[0], "20250401"); // Datum
        assert_eq!(fields[1], "09:30:00"); // Zeit
        assert_eq!(fields[2], "AAPL"); // Symbol
        assert_eq!(fields[4], "150.50"); // Preis
        assert_eq!(fields[6], "1000"); // Menge

        // Überprüfe zweite Zeile (Verkauf mit Dezimalstellen)
        let fields: Vec<&str> = output_lines[2].split(',').collect();
        assert_eq!(fields[4], "151.25"); // Preis
        assert_eq!(fields[6], "-500"); // Menge

        // Überprüfe dritte Zeile (Kauf ohne Dezimalstellen)
        let fields: Vec<&str> = output_lines[3].split(',').collect();
        assert_eq!(fields[2], "MSFT"); // Symbol
        assert_eq!(fields[4], "160.00"); // Preis (ohne Dezimalstellen)
        assert_eq!(fields[6], "1000"); // Menge

        // Überprüfe vierte Zeile (Verkauf ohne Dezimalstellen)
        let fields: Vec<&str> = output_lines[4].split(',').collect();
        assert_eq!(fields[3], "stock"); // Asset Type
        assert_eq!(fields[4], "161.00"); // Preis (ohne Dezimalstellen)
        assert_eq!(fields[5], "USD"); // Währung
        assert_eq!(fields[6], "-500"); // Menge

        Ok(())
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    #[cfg(target_os = "windows")]
    {
        if cli.install {
            windows::install_context_menu()?;
            println!("Windows-Kontextmenü wurde erfolgreich installiert!");
            return Ok(());
        }
        if cli.uninstall {
            windows::uninstall_context_menu()?;
            println!("Windows-Kontextmenü wurde erfolgreich entfernt!");
            return Ok(());
        }
    }

    let input_path = Path::new(&cli.input);
    if !input_path.exists() {
        bail!("Eingabedatei existiert nicht: {}", input_path.display());
    }

    let output_path = match cli.output {
        Some(ref path) => Path::new(path).to_path_buf(),
        None => input_path.with_extension("txt.cust.csv"),
    };

    convert_file(input_path, &output_path, cli.verbose)
}
