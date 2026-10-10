//! Cuadro de mando, a guisa de HUD, para conversar con el Compae.
//!
//! Dícese de una interfaz de terminal que delega la síntesis en `worker.py`,
//! con quien departe mediante JSON por líneas a través de stdin/stdout.

use std::f64::consts::TAU;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Constraint::{Length, Min, Percentage};
use ratatui::layout::{Alignment, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Context, Line as CLine, Points};
use ratatui::widgets::{
    Axis, Block, BorderType, Chart, Dataset, Gauge, GraphType, List, ListItem, ListState, Paragraph, Sparkline,
};
use ratatui::{DefaultTerminal, Frame};
use serde_json::{json, Value};
use sysinfo::{Networks, System};

const CYAN: Color = Color::Rgb(0, 210, 255);
const DIM: Color = Color::Rgb(0, 95, 125);
const GOLD: Color = Color::Rgb(255, 190, 60);
const PLACEHOLDER: &str = "Pregúntale al Compae... (o pulsa F3/Tab y háblale)";
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];
const DIAS: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
const MESES: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio",
    "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];
/// Segundos que abarca cada muestra de la envolvente que remite el trabajador.
const HOP: f64 = 0.05;

struct Frase {
    /// Verdadero si la pronuncia el usuario; falso si es cosa del Compae.
    tu: bool,
    text: String,
    path: Option<String>,
    secs: Option<f64>,
    env: Vec<f64>,
    error: Option<String>,
}

impl Frase {
    fn new(tu: bool, text: impl Into<String>) -> Self {
        Self { tu, text: text.into(), path: None, secs: None, env: Vec::new(), error: None }
    }

    fn pending(&self) -> bool {
        !self.tu && self.path.is_none() && self.error.is_none()
    }
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
}

impl Worker {
    fn spawn(redesign: bool) -> io::Result<Self> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        fs::create_dir_all(dir.join("out"))?;
        let mut cmd = Command::new(dir.join("../.venv/Scripts/python.exe"));
        cmd.arg(dir.join("worker.py"))
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Cabe señalar que el parloteo de torch iría a estropear la pantalla; se archiva con decoro.
            .stderr(File::create(dir.join("out/worker.log"))?);
        if redesign {
            cmd.arg("--redesign");
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let v = serde_json::from_str(&line).unwrap_or_else(|_| json!({ "error": line }));
                if tx.send(v).is_err() {
                    return;
                }
            }
            let _ = tx.send(json!({ "error": "El trabajador ha fallecido; consúltese out/worker.log" }));
        });
        Ok(Self { child, stdin, rx })
    }

    fn send(&mut self, v: Value) {
        let _ = writeln!(self.stdin, "{v}");
        let _ = self.stdin.flush();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Stats {
    sys: System,
    nets: Networks,
    last: Instant,
    cpu: Vec<u64>,
    net_in: f64,
    net_out: f64,
    /// Historial de bytes por segundo, entrada y salida, uno por tick.
    net_hist: Vec<(f64, f64)>,
}

impl Stats {
    fn new() -> Self {
        Self {
            sys: System::new(),
            nets: Networks::new_with_refreshed_list(),
            last: Instant::now(),
            cpu: vec![0; 120],
            net_in: 0.0,
            net_out: 0.0,
            net_hist: vec![(0.0, 0.0); 120],
        }
    }

    fn tick(&mut self) {
        let dt = self.last.elapsed().as_secs_f64();
        if dt < 1.0 {
            return;
        }
        self.last = Instant::now();
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.nets.refresh(true);
        let (rx, tx) = self.nets.iter().fold((0, 0), |(r, t), (_, d)| (r + d.received(), t + d.transmitted()));
        self.net_in = rx as f64 / dt;
        self.net_out = tx as f64 / dt;
        self.net_hist.remove(0);
        self.net_hist.push((self.net_in, self.net_out));
        self.cpu.remove(0);
        self.cpu.push(self.sys.global_cpu_usage() as u64);
    }
}

struct App {
    worker: Worker,
    stats: Stats,
    status: String,
    ready: bool,
    listening: bool,
    /// Aviso efímero para la cabecera, con su instante de nacimiento.
    notice: Option<(String, Instant)>,
    /// Instante de la última pregunta, para dar fe de la espera.
    asked: Instant,
    input: String,
    frases: Vec<Frase>,
    list: ListState,
    started: Instant,
    /// Envolvente de lo que suena ahora mismo y el instante en que empezó.
    env: Vec<f64>,
    play_start: Instant,
    /// Rótulo de la cabecera, que el trabajador remite desde PIRATA_TITULO.
    title: String,
    /// Sonoridad reciente del micrófono (una muestra cada 50 ms) mientras se escucha.
    mic: Vec<f64>,
    /// Pestaña del panel izquierdo: 0 la charla, 1 el tráfico de red.
    tab: usize,
    /// Zonas clicables del último fotograma, con la tecla que emulan.
    hits: Vec<(Rect, KeyCode)>,
}

impl App {
    fn run(&mut self, term: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            while let Ok(v) = self.worker.rx.try_recv() {
                self.on_event(v);
            }
            self.stats.tick();
            term.draw(|f| self.draw(f))?;
            if !event::poll(Duration::from_millis(33))? {
                continue;
            }
            // Un clic se traduce, con toda cortesía, en la tecla equivalente; así no se duplica la lógica.
            let code = match event::read()? {
                // En Windows llegan también las teclas al soltarse; ciertamente sobran.
                Event::Key(k) if k.kind == KeyEventKind::Press => k.code,
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    let at = ratatui::layout::Position::new(m.column, m.row);
                    match self.hits.iter().find(|(r, _)| r.contains(at)) {
                        Some((_, code)) => *code,
                        None => continue,
                    }
                }
                _ => continue,
            };
            match code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Enter => self.ask(),
                KeyCode::F(3) | KeyCode::Tab => self.listen(),
                KeyCode::Char(c) => self.input.push(c),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Up => self.list.select_previous(),
                KeyCode::Down => self.list.select_next(),
                KeyCode::F(2) => self.replay(),
                KeyCode::F(4) => self.clear(),
                KeyCode::Left | KeyCode::Right | KeyCode::F(6) => self.tab = 1 - self.tab,
                KeyCode::F(7) => self.tab = 0,
                KeyCode::F(8) => self.tab = 1,
                KeyCode::F(5) => self.redesign()?,
                _ => {}
            }
        }
    }

    fn on_event(&mut self, v: Value) {
        let env: Vec<f64> = v["env"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        if let Some(s) = v["status"].as_str() {
            self.ready = s == "ready";
            if let Some(t) = v["title"].as_str() {
                self.title = t.into();
            }
            self.status = if self.ready { "Listo, quillo".into() } else { s.into() };
            if self.ready {
                self.start_playback(env);
            }
        } else if let Some(m) = v["mic"].as_f64() {
            self.mic.push(m);
            if self.mic.len() > 400 {
                self.mic.drain(..200);
            }
        } else if let Some(l) = v["listening"].as_bool() {
            self.listening = l;
            self.mic.clear();
        } else if let Some(h) = v["heard"].as_str() {
            // Lo oído se intercala justo antes de la respuesta que aguarda el Compae.
            let at = self.frases.iter().position(Frase::pending).unwrap_or(self.frases.len());
            self.frases.insert(at, Frase::new(true, h));
        } else if let Some(r) = v["reply"].as_str() {
            if let Some(f) = self.frases.iter_mut().find(|f| f.pending()) {
                f.text = r.into();
            }
        } else if v["chunk"].is_number() {
            if let Some(f) = self.frases.iter_mut().find(|f| f.pending()) {
                f.secs = v["secs"].as_f64();
            }
            self.start_playback(env);
        } else if let Some(p) = v["done"].as_str() {
            // Lo dicho ya sonó frase a frase; la envolvente entera se guarda solo para repetirlo con F2.
            if let Some(f) = self.frases.iter_mut().find(|f| f.pending()) {
                f.path = Some(p.into());
                f.secs = v["secs"].as_f64();
                f.env = env;
            }
        } else if let Some(e) = v["error"].as_str() {
            self.status = format!("Error: {e}");
            if let Some(f) = self.frases.iter_mut().find(|f| f.pending()) {
                f.error = Some(e.into());
            }
        }
    }

    fn start_playback(&mut self, env: Vec<f64>) {
        self.env = env;
        self.play_start = Instant::now();
    }

    /// Sonoridad (0..1) del instante presente de la reproducción.
    fn level(&self) -> f64 {
        let i = (self.play_start.elapsed().as_secs_f64() / HOP) as usize;
        self.env.get(i).copied().unwrap_or(0.0)
    }

    fn busy(&self) -> bool {
        !self.ready || self.frases.iter().any(Frase::pending)
    }

    /// Pregunta al Compae; con un `!` delante, en cambio, este se limita a recitar el texto.
    fn ask(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(literal) = text.strip_prefix('!') {
            self.worker.send(json!({ "cmd": "say", "text": literal.trim() }));
            self.frases.push(Frase::new(false, literal.trim()));
        } else {
            self.worker.send(json!({ "cmd": "ask", "text": text }));
            self.frases.push(Frase::new(true, text));
            self.frases.push(Frase::new(false, "…"));
        }
        self.asked = Instant::now();
        self.list.select(Some(self.frases.len() - 1));
        self.input.clear();
    }

    fn listen(&mut self) {
        // Mientras el Compae delibera, el trabajador no atiende; se evita así encolar un micro abierto.
        let veto = if !self.ready {
            Some("El Compae aún se está despertando")
        } else if !self.listening && self.busy() {
            Some("El Compae está liado; espera a que acabe de hablar")
        } else {
            None
        };
        if let Some(v) = veto {
            self.notice = Some((v.into(), Instant::now()));
            return;
        }
        self.worker.send(json!({ "cmd": "listen" }));
        if self.listening {
            self.asked = Instant::now();
            self.frases.push(Frase::new(false, "…"));
            self.list.select(Some(self.frases.len() - 1));
        }
        self.listening = !self.listening;
    }

    fn replay(&mut self) {
        let Some(f) = self.list.selected().and_then(|i| self.frases.get(i)) else { return };
        let Some(path) = f.path.clone() else { return };
        let env = f.env.clone();
        self.worker.send(json!({ "cmd": "play", "path": path }));
        self.start_playback(env);
    }

    fn redesign(&mut self) -> io::Result<()> {
        for f in self.frases.iter_mut().filter(|f| f.pending()) {
            f.error = Some("cancelada por cambio de voz".into());
        }
        self.worker = Worker::spawn(true)?;
        self.ready = false;
        self.listening = false;
        self.status = "Diseñando otra voz...".into();
        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        let [header, body, input, help] = Layout::vertical([Length(1), Min(10), Length(3), Length(1)]).areas(f.area());
        let [left, center, right] = Layout::horizontal([Percentage(28), Min(30), Percentage(26)]).areas(body);

        self.draw_header(f, header);

        let [clock, log] = Layout::vertical([Length(5), Min(3)]).areas(left);
        self.draw_clock(f, clock);
        self.draw_left(f, log);

        let [rings, voice] = Layout::vertical([Min(8), Length(6)]).areas(center);
        self.draw_rings(f, rings);
        self.draw_voice(f, voice);

        self.draw_system(f, right);
        self.draw_input(f, input);

        f.render_widget(
            Line::from(" Enter preguntar · !texto recitar · F3/Tab micro · ↑↓ elegir · F2 repetir · F4 borrar · ←→ chat/red · F5 nueva voz · Esc salir").fg(DIM),
            help,
        );
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let spin = SPINNER[(self.started.elapsed().as_millis() / 150) as usize % SPINNER.len()];
        let queued = self.frases.iter().filter(|f| f.pending()).count();
        let notice = self.notice.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(3));
        let status = match (self.ready && !self.listening, queued) {
            _ if self.listening => "● Escuchando... (F3/Tab pa terminar)".red().bold(),
            _ if notice.is_some() => format!("⚠ {}", notice.unwrap().0).fg(GOLD).bold(),
            (false, _) => format!("{spin} {}", self.status).fg(GOLD),
            (true, 0) => format!("● {}", self.status).green(),
            (true, n) => {
                let speaking = self.frases.iter().any(|f| f.pending() && f.secs.is_some());
                let what = if speaking { "Hablando" } else { "Generando" };
                let cola = if n > 1 { format!(" · {n} en cola") } else { String::new() };
                format!("{spin} {what}... {}s{cola}", self.asked.elapsed().as_secs()).fg(CYAN)
            }
        };
        let title = format!(" ◉ {} ", self.title.to_uppercase());
        let line = Line::from(vec![title.fg(Color::Black).bg(CYAN).bold(), "  ".into(), status]);
        f.render_widget(line, area);
    }

    fn draw_clock(&self, f: &mut Frame, area: Rect) {
        let now = Local::now();
        let fecha = format!(
            "{}, {} de {}",
            DIAS[now.weekday().num_days_from_monday() as usize],
            now.day(),
            MESES[now.month0() as usize]
        );
        let text = vec![
            Line::from(now.format("%H:%M:%S").to_string().fg(Color::White).bold()),
            Line::from(""),
            Line::from(fecha.fg(CYAN)),
        ];
        f.render_widget(Paragraph::new(text).alignment(Alignment::Center).block(panel("HORA LOCAL")), area);
    }

    /// Borra la charla: la bitácora aquí y la memoria del Compae en el trabajador.
    fn clear(&mut self) {
        self.worker.send(json!({ "cmd": "reset" }));
        self.frases.retain(Frase::pending);
        self.list.select(None);
        self.notice = Some(("Conversación borrada, borrón y cuenta nueva".into(), Instant::now()));
    }

    fn draw_left(&mut self, f: &mut Frame, area: Rect) {
        let tab = |name: &str, on: bool| {
            if on { format!(" {name} ").fg(Color::Black).bg(CYAN).bold() } else { format!(" {name} ").fg(DIM) }
        };
        let title = Line::from(vec![" ".into(), tab("CHAT", self.tab == 0), "│".fg(DIM), tab("RED", self.tab == 1), " ←→ ".fg(DIM)]);
        const BORRAR: &str = " ✗ Borrar ";
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(DIM))
            .title(title)
            .title(Line::from(BORRAR.fg(Color::Black).bg(GOLD).bold()).right_aligned());
        // Cabe señalar que las posiciones se calculan a mano: el título arranca tras la esquina del borde.
        let row = |x: u16, w: u16| Rect::new(x, area.y, w, 1);
        let borrar_w = BORRAR.chars().count() as u16;
        self.hits = vec![
            (row(area.x + 2, 6), KeyCode::F(7)),
            (row(area.x + 9, 5), KeyCode::F(8)),
            (row(area.right().saturating_sub(1 + borrar_w), borrar_w), KeyCode::F(4)),
        ];
        let inner = block.inner(area);
        f.render_widget(block, area);
        if self.tab == 0 {
            self.draw_log(f, inner);
        } else {
            self.draw_net(f, inner);
        }
    }

    fn draw_net(&self, f: &mut Frame, area: Rect) {
        let hist = &self.stats.net_hist;
        let inbound: Vec<(f64, f64)> = hist.iter().enumerate().map(|(i, h)| (i as f64, h.0)).collect();
        let outbound: Vec<(f64, f64)> = hist.iter().enumerate().map(|(i, h)| (i as f64, h.1)).collect();
        let top = hist.iter().map(|h| h.0.max(h.1)).fold(1024.0, f64::max) * 1.2;
        let datasets = vec![
            Dataset::default().name(format!("IN  {}", human(self.stats.net_in))).marker(Marker::Braille)
                .graph_type(GraphType::Line).style(Style::new().fg(CYAN)).data(&inbound),
            Dataset::default().name(format!("OUT {}", human(self.stats.net_out))).marker(Marker::Braille)
                .graph_type(GraphType::Line).style(Style::new().fg(GOLD)).data(&outbound),
        ];
        let chart = Chart::new(datasets)
            .x_axis(Axis::default().bounds([0.0, (hist.len() - 1) as f64]).style(Style::new().fg(DIM))
                .labels(["-2 min".fg(DIM), "ahora".fg(DIM)]))
            .y_axis(Axis::default().bounds([0.0, top]).style(Style::new().fg(DIM))
                .labels(["0".fg(DIM), human(top).fg(DIM)]))
            .legend_position(Some(ratatui::widgets::LegendPosition::TopLeft));
        f.render_widget(chart, area);
    }

    fn draw_log(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .frases
            .iter()
            .map(|fr| match (fr.tu, &fr.secs, &fr.error) {
                (true, ..) => ListItem::new(Line::from(vec!["tú ".fg(DIM), fr.text.clone().fg(Color::White)])),
                (_, _, Some(e)) => ListItem::new(format!("◆  ✗ {e}")).red(),
                (_, Some(s), _) => {
                    ListItem::new(Line::from(vec!["◆  ".fg(GOLD), fr.text.clone().fg(CYAN), format!(" {s:.1}s").fg(DIM)]))
                }
                _ => ListItem::new(Line::from(vec!["◆  ".fg(GOLD), fr.text.clone().fg(GOLD).italic()])),
            })
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().fg(Color::Black).bg(CYAN).add_modifier(Modifier::BOLD));
        f.render_stateful_widget(list, area, &mut self.list);
    }

    fn draw_rings(&self, f: &mut Frame, area: Rect) {
        let t = self.started.elapsed().as_secs_f64();
        let lvl = self.level();
        let speed = if self.busy() { 2.2 } else { 0.5 };
        // Las celdas del terminal miden el doble de alto que de ancho; se compensa para que el círculo no sea un huevo.
        let xr = 50.0 * area.width as f64 / (area.height.max(1) as f64 * 2.0);
        let canvas = Canvas::default()
            .block(panel("NÚCLEO"))
            .marker(Marker::Braille)
            .x_bounds([-xr, xr])
            .y_bounds([-50.0, 50.0])
            .paint(move |ctx| {
                ctx.draw(&Circle { x: 0.0, y: 0.0, radius: 46.0, color: DIM });
                for (r, n, dir, color) in [(40.0, 3, 1.0, CYAN), (33.0, 6, -1.6, DIM), (27.0, 2, 2.4, GOLD)] {
                    for k in 0..n {
                        let a0 = t * speed * dir + k as f64 * TAU / n as f64;
                        arc(ctx, r, a0, a0 + TAU / (n as f64 * 2.0), color);
                    }
                }
                let pulse = 10.0 + 12.0 * lvl;
                ctx.draw(&Circle { x: 0.0, y: 0.0, radius: pulse, color: if lvl > 0.05 { Color::White } else { CYAN } });
                ctx.print(-2.0, 2.0, "◉".fg(Color::White).bold());
            });
        f.render_widget(canvas, area);
    }

    fn draw_voice(&self, f: &mut Frame, area: Rect) {
        // Osciloscopio: línea en calma que se agita en señal con la sonoridad. Cian si habla el Compae
        // (su envolvente, al compás de la reproducción); rojo si escucha el micro, en vivo.
        const WINDOW: f64 = 60.0; // muestras de 50 ms a la vista: tres segundos
        let (levels, head, color) = if self.listening {
            (&self.mic, self.mic.len() as f64, Color::Red)
        } else {
            (&self.env, self.play_start.elapsed().as_secs_f64() / HOP, CYAN)
        };
        let amp = |x: f64| {
            let j = head - WINDOW + x * WINDOW;
            if j < 0.0 { 0.0 } else { levels.get(j as usize).copied().unwrap_or(0.0) }
        };
        let t = self.started.elapsed().as_secs_f64();
        let n = (area.width.saturating_sub(2) as usize * 2).max(2);
        let pts: Vec<(f64, f64)> = (0..=n)
            .map(|k| {
                let x = k as f64 / n as f64;
                // Dos senos que no casan dan aire de voz; el arrastre por t la hace fluir.
                let wave = (x * 70.0 - t * 14.0).sin() * 0.7 + (x * 23.0 + t * 5.0).sin() * 0.3;
                (x, amp(x) * wave)
            })
            .collect();
        let canvas = Canvas::default()
            .block(panel("VOZ"))
            .marker(Marker::Braille)
            .x_bounds([0.0, 1.0])
            .y_bounds([-1.05, 1.05])
            .paint(move |ctx| {
                ctx.draw(&CLine { x1: 0.0, y1: 0.0, x2: 1.0, y2: 0.0, color: DIM });
                for w in pts.windows(2) {
                    ctx.draw(&CLine { x1: w[0].0, y1: w[0].1, x2: w[1].0, y2: w[1].1, color });
                }
            });
        f.render_widget(canvas, area);
    }

    fn draw_system(&self, f: &mut Frame, area: Rect) {
        let block = panel("SISTEMA");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [cpu, ram, _, net_in, net_out, _, hist] =
            Layout::vertical([Length(1), Length(1), Length(1), Length(1), Length(1), Length(1), Min(1)]).areas(inner);

        let cpu_now = *self.stats.cpu.last().unwrap_or(&0);
        let mem = &self.stats.sys;
        let ram_pct = 100 * mem.used_memory() / mem.total_memory().max(1);
        f.render_widget(gauge("CPU", cpu_now), cpu);
        f.render_widget(gauge("RAM", ram_pct), ram);
        f.render_widget(Line::from(vec!["IN  ".fg(DIM), human(self.stats.net_in).fg(Color::White)]), net_in);
        f.render_widget(Line::from(vec!["OUT ".fg(DIM), human(self.stats.net_out).fg(Color::White)]), net_out);

        let n = hist.width as usize;
        let cpu_hist = &self.stats.cpu[self.stats.cpu.len().saturating_sub(n)..];
        f.render_widget(Sparkline::default().data(cpu_hist).max(100).style(Style::new().fg(DIM)), hist);
    }

    fn draw_input(&self, f: &mut Frame, area: Rect) {
        let text = if self.input.is_empty() {
            Line::from(vec!["❯ ".fg(CYAN), PLACEHOLDER.fg(DIM).italic()])
        } else {
            Line::from(vec!["❯ ".fg(CYAN), self.input.as_str().fg(Color::White)])
        };
        f.render_widget(Paragraph::new(text).block(panel("PREGUNTA")), area);
        // ponytail: el cursor cuenta caracteres, no anchura; los emojis lo desplazarán un pelín
        f.set_cursor_position((area.x + 3 + self.input.chars().count() as u16, area.y + 1));
    }
}

fn panel(title: &str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM))
        .title(Line::from(vec![Span::from(format!(" {title} ")).fg(CYAN).bold(), "● ".green()]))
}

fn gauge(label: &str, pct: u64) -> Gauge<'static> {
    Gauge::default()
        .gauge_style(Style::new().fg(CYAN).bg(Color::Rgb(0, 30, 40)))
        .percent(pct.min(100) as u16)
        .label(format!("{label} {pct}%"))
}

fn arc(ctx: &mut Context, r: f64, a0: f64, a1: f64, color: Color) {
    let coords: Vec<(f64, f64)> = (0..=60)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / 60.0;
            (r * a.cos(), r * a.sin())
        })
        .collect();
    ctx.draw(&Points { coords: &coords, color });
}

fn human(bps: f64) -> String {
    match bps {
        b if b >= 1e6 => format!("{:.1} MB/s", b / 1e6),
        b if b >= 1e3 => format!("{:.1} KB/s", b / 1e3),
        b => format!("{b:.0} B/s"),
    }
}

fn main() -> io::Result<()> {
    let mut app = App {
        worker: Worker::spawn(false)?,
        stats: Stats::new(),
        status: "Despertando al Compae...".into(),
        ready: false,
        listening: false,
        notice: None,
        asked: Instant::now(),
        input: String::new(),
        frases: Vec::new(),
        list: ListState::default(),
        started: Instant::now(),
        env: Vec::new(),
        play_start: Instant::now(),
        title: "Compae".into(),
        mic: Vec::new(),
        tab: 0,
        hits: Vec::new(),
    };
    let mut term = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let res = app.run(&mut term);
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn pinta_el_hud() {
        let mut app = App {
            worker: Worker::spawn(false).unwrap(),
            stats: Stats::new(),
            status: "Listo, quillo".into(),
            ready: true,
            listening: false,
            notice: None,
            asked: Instant::now(),
            input: "¿Dónde está el tesoro?".into(),
            frases: vec![
                Frase::new(true, "¿Hay tesoro?"),
                Frase { path: Some("x.wav".into()), secs: Some(2.1), ..Frase::new(false, "Tengo medio plan.") },
            ],
            list: ListState::default().with_selected(Some(0)),
            started: Instant::now(),
            env: (0..200).map(|i| ((i as f64) / 5.0).sin().abs()).collect(),
            play_start: Instant::now() - Duration::from_secs(3),
            title: "Compae".into(),
            mic: Vec::new(),
            tab: 0,
            hits: Vec::new(),
        };
        let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let buf = term.backend().buffer();
        let text: String = buf.content.iter().map(|c| c.symbol()).collect();
        for needle in ["COMPAE", "HORA LOCAL", "CHAT", "RED", "NÚCLEO", "VOZ", "SISTEMA", "PREGUNTA", "Tengo medio plan."] {
            assert!(text.contains(needle), "falta {needle}");
        }
        println!("{}", term.backend());
        app.tab = 1;
        term.draw(|f| app.draw(f)).unwrap();
        let red: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(red.contains("IN ") && red.contains("ahora"), "falta la gráfica de red");
        // Los rótulos clicables han de caer justo encima de su texto.
        let at = |r: Rect| -> String { (r.x..r.right()).map(|x| term.backend().buffer()[(x, r.y)].symbol().to_string()).collect() };
        let labels: Vec<String> = app.hits.iter().map(|(r, _)| at(*r)).collect();
        assert_eq!(labels, [" CHAT ", " RED ", " ✗ Borrar "]);
    }
}
