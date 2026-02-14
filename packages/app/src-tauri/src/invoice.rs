use printpdf::*;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

#[derive(Debug)]
pub struct InvoiceEntry {
    pub date: String,
    pub hours: f64,
    pub rate: f64,
    pub amount: f64,
}

#[derive(Debug)]
pub struct InvoiceData {
    pub invoice_number: String,
    pub invoice_date: String,
    pub business_name: String,
    pub business_email: Option<String>,
    pub project_name: String,
    pub entries: Vec<InvoiceEntry>,
    pub subtotal: f64,
    pub tax_rate: f64,
    pub tax_amount: f64,
    pub total: f64,
}

pub fn generate_invoice_pdf(data: InvoiceData, output_path: PathBuf) -> Result<String, String> {
    // Create PDF document
    let (doc, page1, layer1) = PdfDocument::new(
        format!("Invoice #{}", data.invoice_number),
        Mm(210.0),  // A4 width
        Mm(297.0),  // A4 height
        "Layer 1",
    );

    let current_layer = doc.get_page(page1).get_layer(layer1);

    // Load fonts
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).map_err(|e| e.to_string())?;
    let font_regular = doc.add_builtin_font(BuiltinFont::Helvetica).map_err(|e| e.to_string())?;

    let mut y_position = 270.0; // Start from top (A4 is 297mm height)

    // Header - Invoice Title
    current_layer.use_text(
        "INVOICE",
        24.0,
        Mm(20.0),
        Mm(y_position),
        &font_bold,
    );

    y_position -= 10.0;

    // Invoice date (right aligned)
    current_layer.use_text(
        format!("Date: {}", data.invoice_date),
        10.0,
        Mm(140.0),
        Mm(y_position),
        &font_regular,
    );

    y_position -= 15.0;

    // Business info (from)
    current_layer.use_text("FROM:", 11.0, Mm(20.0), Mm(y_position), &font_bold);
    y_position -= 6.0;

    current_layer.use_text(&data.business_name, 10.0, Mm(20.0), Mm(y_position), &font_regular);
    y_position -= 5.0;

    if let Some(ref email) = data.business_email {
        if !email.is_empty() {
            current_layer.use_text(email, 10.0, Mm(20.0), Mm(y_position), &font_regular);
            y_position -= 5.0;
        }
    }

    y_position -= 10.0;

    // Client info (to) - using project name
    current_layer.use_text("BILL TO:", 11.0, Mm(20.0), Mm(y_position), &font_bold);
    y_position -= 6.0;

    current_layer.use_text(&data.project_name, 10.0, Mm(20.0), Mm(y_position), &font_regular);
    y_position -= 5.0;

    y_position -= 5.0;

    // Table header
    let line = Line {
        points: vec![
            (Point::new(Mm(20.0), Mm(y_position)), false),
            (Point::new(Mm(190.0), Mm(y_position)), false),
        ],
        is_closed: false,
    };
    current_layer.add_line(line);

    y_position -= 5.0;

    current_layer.use_text("Period", 10.0, Mm(20.0), Mm(y_position), &font_bold);
    current_layer.use_text("Hours", 10.0, Mm(130.0), Mm(y_position), &font_bold);
    current_layer.use_text("Rate", 10.0, Mm(155.0), Mm(y_position), &font_bold);
    current_layer.use_text("Amount", 10.0, Mm(175.0), Mm(y_position), &font_bold);

    y_position -= 5.0;

    let line = Line {
        points: vec![
            (Point::new(Mm(20.0), Mm(y_position)), false),
            (Point::new(Mm(190.0), Mm(y_position)), false),
        ],
        is_closed: false,
    };
    current_layer.add_line(line);

    y_position -= 6.0;

    // Entries
    for entry in &data.entries {
        if y_position < 50.0 {
            // Need new page
            // For simplicity, we'll just stop here
            // In production, you'd create a new page
            break;
        }

        current_layer.use_text(&entry.date, 9.0, Mm(20.0), Mm(y_position), &font_regular);
        current_layer.use_text(format!("{:.2}", entry.hours), 9.0, Mm(130.0), Mm(y_position), &font_regular);
        current_layer.use_text(format!("${:.2}", entry.rate), 9.0, Mm(155.0), Mm(y_position), &font_regular);
        current_layer.use_text(format!("${:.2}", entry.amount), 9.0, Mm(175.0), Mm(y_position), &font_regular);

        y_position -= 5.0;
    }

    y_position -= 5.0;

    // Bottom line
    let line = Line {
        points: vec![
            (Point::new(Mm(20.0), Mm(y_position)), false),
            (Point::new(Mm(190.0), Mm(y_position)), false),
        ],
        is_closed: false,
    };
    current_layer.add_line(line);

    y_position -= 10.0;

    // Totals (right aligned)
    current_layer.use_text("Subtotal:", 10.0, Mm(150.0), Mm(y_position), &font_regular);
    current_layer.use_text(format!("${:.2}", data.subtotal), 10.0, Mm(170.0), Mm(y_position), &font_regular);

    if data.tax_rate > 0.0 {
        y_position -= 6.0;
        current_layer.use_text(
            format!("Tax ({}%):", data.tax_rate),
            10.0,
            Mm(150.0),
            Mm(y_position),
            &font_regular,
        );
        current_layer.use_text(format!("${:.2}", data.tax_amount), 10.0, Mm(170.0), Mm(y_position), &font_regular);
    }

    y_position -= 8.0;

    current_layer.use_text("TOTAL:", 11.0, Mm(150.0), Mm(y_position), &font_bold);
    current_layer.use_text(format!("${:.2}", data.total), 11.0, Mm(170.0), Mm(y_position), &font_bold);

    // Save PDF
    let file = File::create(&output_path).map_err(|e| format!("Failed to create file: {}", e))?;
    let mut buf_writer = BufWriter::new(file);
    doc.save(&mut buf_writer).map_err(|e| format!("Failed to save PDF: {}", e))?;

    Ok(output_path.to_string_lossy().to_string())
}

pub fn get_invoices_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Could not find home directory");
    let protimer_dir = home.join(".protimer").join("invoices");

    if !protimer_dir.exists() {
        fs::create_dir_all(&protimer_dir).expect("Failed to create invoices directory");
    }

    protimer_dir
}

pub fn get_project_invoices_dir(project_name: &str) -> PathBuf {
    let invoices_dir = get_invoices_dir();

    // Sanitize project name for filesystem (replace invalid chars)
    let safe_name = project_name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>();

    let project_dir = invoices_dir.join(safe_name);

    if !project_dir.exists() {
        fs::create_dir_all(&project_dir).expect("Failed to create project invoices directory");
    }

    project_dir
}
