//! emlbox-gui: просмотрщик .eml-контейнеров (egui/eframe).
//!
//! Вкладки: Sections (базовые секции, декодированные), Deltas (цепочки
//! писателей), KV (если секция/состояние — JSON-таблица), Verify.
//! Запуск: emlbox-gui [file.eml]

use emlbox::format::{block_header, hash_bytes};
use emlbox::reader::EmlBox;
use eframe::egui;

fn main() -> eframe::Result {
    let path = std::env::args().nth(1).unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "emlbox-gui",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, path)))),
    )
}

struct App {
    path: String,
    b: Option<EmlBox>,
    error: Option<String>,
    tab: Tab,
    selected: Option<Selected>,
    doc_input: String,
    doc_revert: String,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Sections,
    Deltas,
    Kv,
    Doc,
    Verify,
}

#[derive(PartialEq, Clone)]
enum Selected {
    Section(String),
    Delta(usize),
    Table(String),
}

impl App {
    fn new(_cc: &eframe::CreationContext<'_>, path: String) -> Self {
        let mut a = App {
            path,
            b: None,
            error: None,
            tab: Tab::Sections,
            selected: None,
            doc_input: String::new(),
            doc_revert: String::new(),
        };
        a.open();
        a
    }

    fn open(&mut self) {
        self.error = None;
        match EmlBox::open(std::path::Path::new(&self.path)) {
            Ok(b) => {
                self.b = Some(b);
                self.selected = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Контейнер:");
                let resp = ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(420.0));
                if ui.button("Open").clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                    self.open();
                }
                ui.separator();
                if let Some(b) = &self.b {
                    ui.label(format!("entity: {}", b.entity().unwrap_or_default()));
                    ui.label(format!("{} секций, {} дельт", b.sections.len(), b.tail_entries().len()));
                }
            });
            if let Some(e) = &self.error {
                ui.colored_label(egui::Color32::RED, e);
            }
        });

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Sections, "Sections");
                ui.selectable_value(&mut self.tab, Tab::Deltas, "Deltas");
                ui.selectable_value(&mut self.tab, Tab::Kv, "KV");
                ui.selectable_value(&mut self.tab, Tab::Doc, "Doc");
                ui.selectable_value(&mut self.tab, Tab::Verify, "Verify");
            });
        });

        // ---- данные до панелей (borrow-free для closure) ----
        let sections: Vec<(String, u64, String)> = self
            .b
            .as_ref()
            .map(|b| b.sections.iter().map(|s| (s.id.clone(), s.len, s.enc.clone())).collect())
            .unwrap_or_default();
        let deltas: Vec<(String, u64, String)> = self
            .b
            .as_ref()
            .map(|b| b.tail_entries().iter().map(|e| (e.writer.clone(), e.seq, e.hash[..12].to_string())).collect())
            .unwrap_or_default();
        let kv_tables: Vec<String> = self
            .b
            .as_ref()
            .map(|b| b.sections.iter().filter(|s| s.ct.contains("json")).map(|s| s.id.clone()).collect())
            .unwrap_or_default();
        let mut new_selected = self.selected.clone();
        let tab = self.tab;

        egui::SidePanel::left("nav").default_width(300.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                match tab {
                    Tab::Sections => {
                        ui.heading("Sections");
                        for (id, len, enc) in &sections {
                            let label = format!("{id} ({len}b, {enc})");
                            if ui.selectable_label(new_selected == Some(Selected::Section(id.clone())), label).clicked() {
                                new_selected = Some(Selected::Section(id.clone()));
                            }
                        }
                    }
                    Tab::Deltas => {
                        ui.heading("Deltas (по писателям)");
                        let mut writers: Vec<(String, Vec<usize>)> = Vec::new();
                        for (i, (w, _, _)) in deltas.iter().enumerate() {
                            match writers.iter_mut().find(|(wn, _)| wn == w) {
                                Some((_, v)) => v.push(i),
                                None => writers.push((w.clone(), vec![i])),
                            }
                        }
                        for (w, idxs) in writers {
                            ui.label(format!("writer {w} ({})", idxs.len()));
                            for i in idxs {
                                let (_, seq, h) = &deltas[i];
                                let label = format!("  #{}.{} {}", w, seq, h);
                                if ui.selectable_label(new_selected == Some(Selected::Delta(i)), label).clicked() {
                                    new_selected = Some(Selected::Delta(i));
                                }
                            }
                        }
                    }
                    Tab::Kv => {
                        ui.heading("KV-таблицы");
                        for t in &kv_tables {
                            if ui.selectable_label(new_selected == Some(Selected::Table(t.clone())), t).clicked() {
                                new_selected = Some(Selected::Table(t.clone()));
                            }
                        }
                    }
                    Tab::Doc => {
                        ui.heading("Документ (doc)");
                        ui.label("Строки документа в таблице doc/lines");
                        if ui.button("Показать документ").clicked() {
                            new_selected = Some(Selected::Table("doc".to_string()));
                        }
                        ui.separator();
                        ui.label("Добавить строку:");
                        ui.add(egui::TextEdit::singleline(&mut self.doc_input).desired_width(200.0));
                        if ui.button("Add").clicked() && !self.doc_input.trim().is_empty() {
                            let text = self.doc_input.clone();
                            let r = emlbox::kv::add(std::path::Path::new(&self.path), "gui", "doc", "lines", serde_json::Value::String(text), None);
                            match r {
                                Ok(_) => {
                                    self.open();
                                    self.doc_input.clear();
                                }
                                Err(e) => self.error = Some(e),
                            }
                        }
                        ui.separator();
                        ui.label("Откатить N дельт:");
                        ui.add(egui::TextEdit::singleline(&mut self.doc_revert).desired_width(60.0));
                        if ui.button("Revert").clicked() {
                            if let Ok(n) = self.doc_revert.trim().parse::<usize>() {
                                let total = self.b.as_ref().map(|b| b.tail_entries().len()).unwrap_or(0);
                                let r = emlbox::repair::truncate_blocks(std::path::Path::new(&self.path), total.saturating_sub(n));
                                match r {
                                    Ok(_) => {
                                        self.open();
                                        self.doc_revert.clear();
                                    }
                                    Err(e) => self.error = Some(e),
                                }
                            }
                        }
                    }
                    Tab::Verify => {
                        ui.heading("Verify");
                        if ui.button("Проверить целостность").clicked() {
                            match emlbox::verify::verify(std::path::Path::new(&self.path)) {
                                Ok(issues) => {
                                    if issues.is_empty() {
                                        ui.colored_label(egui::Color32::GREEN, "OK: base hash и все цепочки писателей целы");
                                    } else {
                                        for i in &issues {
                                            ui.colored_label(egui::Color32::RED, i);
                                        }
                                    }
                                    ui.separator();
                                }
                                Err(e) => {
                                    ui.colored_label(egui::Color32::RED, e);
                                    ui.separator();
                                }
                            }
                        }
                    }
                }
            });
        });
        self.selected = new_selected;

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(b) = &self.b {
                    match &self.selected {
                        Some(Selected::Section(id)) => {
                            if let Some(data) = b.section(id) {
                                show_bytes(ui, id, &data);
                            } else {
                                ui.colored_label(egui::Color32::RED, "не удалось декодировать (нужен ключ?)");
                            }
                        }
                        Some(Selected::Delta(i)) => {
                            let e = &b.tail_entries()[*i];
                            if let Ok(block) = emlbox::format::slice(&b.mmap, e.off, e.len) {
                                ui.monospace(format!("writer {}  seq {}  hash {}  off {}  len {}", e.writer, e.seq, e.hash, e.off, e.len));
                                ui.separator();
                                ui.monospace(String::from_utf8_lossy(block));
                            }
                        }
                        Some(Selected::Table(t)) => {
                            match emlbox::kv::table(b, t) {
                                Ok(v) => {
                                    if let Some(obj) = v.as_object() {
                                        let mut keys: Vec<&String> = obj.keys().collect();
                                        keys.sort();
                                        for k in keys {
                                            ui.horizontal(|ui| {
                                                ui.monospace(k);
                                                ui.label(obj[k].to_string());
                                            });
                                        }
                                    } else {
                                        ui.label(v.to_string());
                                    }
                                }
                                Err(e) => {
                                    ui.colored_label(egui::Color32::RED, e);
                                    ui.separator();
                                }
                            }
                        }
                        None => {
                            ui.weak("Открой контейнер — слева список секций/дельт");
                        }
                    }
                }
            });
        });
    }
}

fn show_bytes(ui: &mut egui::Ui, id: &str, data: &[u8]) {
    let printable = data.iter().all(|b| *b == b'\n' || *b == b'\r' || *b == b'\t' || (*b >= 0x20 && *b != 0x7f));
    ui.monospace(format!("{id}: {} bytes, sha256 {}", data.len(), &hash_bytes(data)[..16]));
    ui.separator();
    if printable {
        ui.monospace(String::from_utf8_lossy(data).to_string());
    } else {
        // hex-дамп
        for (i, chunk) in data.chunks(16).enumerate().take(200) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let ascii: String = chunk.iter().map(|b| if *b >= 0x20 && *b <= 0x7e { *b as char } else { '.' }).collect();
            ui.monospace(format!("{:08x}  {}  {}", i * 16, hex.join(" "), ascii));
        }
        if data.len() > 3200 {
            ui.weak(format!("... {} байт всего", data.len()));
        }
    }
    let _ = block_header;
}
