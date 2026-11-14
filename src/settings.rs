use crate::imports::{self, Dictionary};

use eframe::egui;

use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
struct Settings {
    preferences: crate::InputPreferences,
    pending: Option<Dictionary>,
    work: Option<Receiver<Result<Dictionary, String>>>,
    message: String,
}
impl eframe::App for Settings {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(rx) = &self.work {
            if let Ok(result) = rx.try_recv() {
                self.work = None;

                match result {
                    Ok(d) => self.pending = Some(d),
                    Err(e) => self.message = e,
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Waypinyin 设置");
            ui.heading("输入体验");
            ui.checkbox(&mut self.preferences.enabled, "启用模糊拼音");
            ui.add_enabled_ui(self.preferences.enabled, |ui| {
                for (i, label) in crate::InputPreferences::LABELS.iter().enumerate() {
                    ui.checkbox(&mut self.preferences.groups[i], *label);
                }
            });
            ui.horizontal(|ui| {
                if ui.button("恢复默认").clicked() {
                    self.preferences = Default::default();
                }
                if ui.button("保存输入设置").clicked() {
                    self.message = match self.preferences.save() {
                        Ok(()) => "已保存，下一次开始输入时生效".into(),
                        Err(e) => e,
                    };
                }
            });
            ui.separator();
            ui.heading("词库管理");
            ui.label("搜狗 scel、微软短语 dat、QQ qpyd/qcel、苹果 plist、TSV / Rime 文本");
            if ui
                .add_enabled(self.work.is_none(), egui::Button::new("选择词库文件…"))
                .clicked()
            {
                let (tx, rx) = mpsc::channel();
                self.work = Some(rx);
                std::thread::spawn(move || {
                    let result = rfd::FileDialog::new()
                        .pick_file()
                        .map_or_else(|| Err("已取消".into()), |p| Dictionary::parse(&p));
                    let _ = tx.send(result);
                });
            }
            if self.work.is_some() {
                ui.spinner();
                ui.label("正在读取词库…");
            }
            if let Some(d) = &self.pending {
                ui.separator();
                ui.label(format!(
                    "{}：{} 条，跳过 {} 条",
                    d.name,
                    d.records.len(),
                    d.skipped
                ));
                for r in d.records.iter().take(8) {
                    let kind = if r.shortcut { "（快捷短语）" } else { "" };
                    ui.label(format!("{} → {}{}", r.code, r.word, kind));
                }
                if ui.button("确认导入").clicked() {
                    self.message = match d.install(&imports::root()) {
                        Ok(()) => "导入成功，下一次开始输入时生效".into(),
                        Err(e) => e,
                    };
                    self.pending = None;
                }
            }
            ui.label(&self.message);
            ui.separator();
            ui.heading("已安装词库");
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Ok(files) = std::fs::read_dir(imports::root()) {
                    for f in files.flatten() {
                        let p = f.path();
                        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                        if ext != "tsv" && ext != "disabled" {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.label(p.file_stem().unwrap_or_default().to_string_lossy());
                            let action = if ext == "tsv" { "停用" } else { "启用" };
                            if ui.button(action).clicked() {
                                let next = if ext == "tsv" { "disabled" } else { "tsv" };
                                if let Err(e) = std::fs::rename(&p, p.with_extension(next)) {
                                    self.message = e.to_string();
                                }
                            }
                            if ui.button("移除").clicked() {
                                if let Err(e) = std::fs::remove_file(&p) {
                                    self.message = e.to_string();
                                }
                            }
                        });
                    }
                }
            });
        });
    }
}
pub fn run() -> eframe::Result {
    eframe::run_native(
        "Waypinyin 词库管理",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();

            if let Ok(out) = std::process::Command::new("fc-match")
                .args(["-f", "%{file}\n%{index}", "sans:lang=zh"])
                .output()
            {
                let matched = String::from_utf8_lossy(&out.stdout);
                let mut lines = matched.lines();
                let path = lines.next().unwrap_or_default();
                let index = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                if let Ok(data) = std::fs::read(path) {
                    let mut font = egui::FontData::from_owned(data);
                    font.index = index;
                    fonts
                        .font_data
                        .insert("chinese".into(), std::sync::Arc::new(font));

                    fonts
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .insert(0, "chinese".into());
                }
            }
            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(Settings {
                preferences: crate::InputPreferences::load(),
                ..Default::default()
            }))
        }),
    )
}
