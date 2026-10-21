# VA-03 — Decisión sobre la transición del avatar

Fecha de consulta: 2026-10-16. Investigación documental; no se instalaron paquetes ni se modificaron manifests o lockfile.

## Decisión

**Recomiendo la opción nativa: conservar Ratatui `Canvas`/estilos y calcular el color interpolado en la animación ya propuesta por VA-02. No añadir TachyonFX ni preparar un A/B con esa dependencia.** La única transición candidata es la habilitación de `mic_enabled`: cian → rojo en 180 ms, con etiqueta y glyph visibles desde el primer frame y corte inmediato si la señal se deshabilita.

El efecto es un solo parámetro de color dentro de `draw_core`, cuya geometría ya se dibuja mediante `Canvas::paint`. La opción nativa expresa ese cambio con una mezcla RGB y el progreso temporal existente. TachyonFX puede animar celdas y limitarse a un área, pero opera después del renderizado sobre el buffer de Ratatui; aquí exigiría transportar y aplicar un efecto con ciclo de vida por cliente para hacer lo mismo, sin aportar una capacidad necesaria para esta transición. Esta recomendación es por simplicidad e integración, no por una comparación visual: VA-06 sigue siendo la inspección que decidiría cualquier afirmación sobre legibilidad o fluidez.

## Comparación para la misma transición

| Aspecto | `Canvas` / estilos existentes | TachyonFX 0.25.2 |
|---|---|---|
| Dibujo | Calcular `p = min(elapsed_ms / 180, 1)` y usar `Color::Rgb` interpolado de cian a rojo al dibujar el anillo en `draw_core`. El mismo `Canvas` representa el resultado en la terminal y en el backend web. | Renderizar el núcleo y luego procesar solo su rectángulo del buffer con un efecto de interpolación de color. El paquete tiene filtros por área/celda y procesa efectos en cada frame. |
| Tiempo y estado | Mantener inicio/progreso de esta única transición en el ciclo de presentación nativo y web; pasar al renderer compartido solo el progreso normalizado. Terminar al llegar a 180 ms; al apagarse `mic_enabled`, cancelar y dibujar el estado apagado de inmediato. | Conservar `Effect`/`EffectManager` entre frames, añadir un efecto al recibir el flanco de activación y pasar el delta del frame a `process`/`process_effects`. Al apagar el micrófono se necesita cancelar/descartar ese efecto de forma explícita. |
| Región | Local por construcción: la mezcla se usa en el anillo de `draw_core`; no altera texto, avisos, llamada, waveform ni controles. | Posible con el área entregada al procesador o `CellFilter::Area`; se debe confirmar que el rectángulo usado por el layout delimita exactamente el núcleo en ambos clientes y que el filtro no alcanza glyphs/avisos. |
| Coste de integración | Sin crate ni cambio de resolución; reutiliza `Canvas`, `Color` y los deltas que ya calculan ambos ciclos. | Añade un motor genérico stateful para una mezcla de color, más ciclo de vida y procesamiento de buffer. La capa que conoce el rectángulo y la instancia persistente no es hoy el renderer común. |
| Uso en WASM | El HUD web usa el mismo `tipsy-ui`/Ratatui y ya recibe `dt` del ciclo basado en `requestAnimationFrame`; el cálculo aritmético del color no necesita reloj de plataforma. | El crate publica compatibilidad WASM. Para una integración compartida habría que habilitar y compilar la configuración WASM adecuada en `tipsy-ui`; la documentación del crate no es una prueba del build ni del resultado visual de esta aplicación. |
| Movimiento reducido | Si más adelante existe una señal, dibujar directamente el color final. VA-02 no encontró preferencia ni API común; no se propone añadirla aquí. | Igual de posible, pero habría que detener/omitir el efecto por señal externa; TachyonFX no resuelve la ausencia actual de configuración. |

## Compatibilidad y candidato evaluado

El registro publicado muestra **TachyonFX 0.25.2**, publicado el **2026-09-06**, con licencia **MIT**, soporte WASM/no-std y dependencia normal de `ratatui-core ^0.1.2`; su página de paquete lista Ratatui 0.30.2 como dependencia de desarrollo. La lista de funciones incluye temporizadores/duración, filtros por celdas o área, y procesamiento por delta después del renderizado. El proyecto tiene `ratatui = 0.30` en `Cargo.toml`, resuelto a `ratatui 0.30.2` y `ratatui-core 0.1.2` en `Cargo.lock`. Por tanto, el requisito base de versión visible coincide; no se observa una incompatibilidad de tipos por versión en esta pareja.

Eso no demuestra que el crate haya sido compilado con las features/targets de este workspace, ni que su salida visual sea idéntica en terminal y Ratzilla. No se hizo resolución de Cargo ni build, de acuerdo con el alcance de VA-03.

Si se reabriera la comparación para el UI compartido por ambos clientes, fijaría **`tachyonfx = 0.25.2`** (versión exacta, sin rango abierto) con `default-features = false, features = ["std", "wasm"]`. Así se desactiva el DSL porque la transición se define en Rust y se habilita la feature WASM publicada, que activa la dependencia opcional `web-time`; `std` habilita soporte de biblioteca estándar. La feature separada `std-duration` cambia el tipo `tachyonfx::Duration` a `std::time::Duration`; no hace falta si se expresa el efecto con los milisegundos que recibe su timer. Para esta transición no hace falta que la biblioteca lea su propio reloj: los dos clientes ya calculan delta de frame y el renderer compartido no debe consultar un reloj. Se evitaría `dsl`/`anpa`; las demás dependencias normales del crate seguirían existiendo. Esta combinación necesita verificación de `cargo tree`/build para ambos targets en una tarea autorizada antes de cualquier integración.

## Dónde viviría el tiempo si se implementa

La señal de activación es únicamente el flanco de `UiState.mic_open` de falso a verdadero, que VA-02 define como petición/habilitación de HUD, no prueba de que el micrófono capturó audio. El progreso de la transición no debe leer `mic_levels`, `voice_envelope`, `voice_pos_ms`, el estado del proveedor ni la reproducción local.

- **Terminal nativo:** una pequeña estructura de presentación junto al ciclo en `tipsy-assistant/src/client.rs::frame_loop`, avanzada con el `dt` ya medido allí. La capa que prepara el frame pasaría al UI compartido el progreso `0..=1` (o un color ya calculado); no añadiría reloj, dispositivo ni socket a `tipsy-ui::render::draw_core`.
- **Navegador:** la estructura equivalente junto al `Shared`/ciclo de frame en `tipsy-web/src/lib.rs`, avanzada con el `dt` que ya se calcula para `requestAnimationFrame`. No usaría `AudioContext.currentTime`: el color sólo depende del flanco de `mic_open` y del delta de presentación. Pasaría el mismo dato escalar al render compartido, sin pasar el objeto `AudioContext`, el WebSocket ni `Playback` al renderer.
- **Renderer compartido:** `tipsy-ui/src/render.rs::draw_core` permanece determinista para iguales `UiState`, área y progreso recibido. Dibuja sólo el color de transición en el anillo del avatar; no gestiona ciclo de vida, mira el reloj ni consume señales de hardware/red.

El manejo final debe respetar VA-02: el texto/glyph cambia inmediatamente; el color interpola por 180 ms; si el valor vuelve a falso por cierre o error de apertura, el render siguiente muestra el estado final apagado sin animar reversa. El estado temporal se libera al completar/cancelar. La duración sigue siendo hipótesis de diseño, no una medida de rendimiento ni una observación de usuario.

## Fuentes y evidencia

Consultadas el 2026-10-16; las páginas de docs.rs/crates.io pueden cambiar para `latest`, por eso se enlaza la versión concreta:

- [Ficha publicada de TachyonFX 0.25.2 en docs.rs](https://docs.rs/crate/tachyonfx/0.25.2): fecha, licencia, dependencias (incluido `ratatui-core ^0.1.2` y Ratatui 0.30.2 de desarrollo), funciones y soporte WASM.
- [Documentación de TachyonFX 0.25.2](https://docs.rs/tachyonfx/0.25.2/tachyonfx/): conceptos stateful, procesamiento después del renderizado, aplicación por frame, filtros de región y lista de features.
- [Manifest empaquetado de TachyonFX 0.25.2](https://docs.rs/crate/tachyonfx/0.25.2/source/Cargo.toml) y [features resueltas](https://docs.rs/crate/tachyonfx/0.25.2/features): versión exacta, feature default (`std` + `dsl`), `dsl`/`anpa` opcional, `wasm`/`web-time` y licencia MIT. La herramienta web no pudo abrir directamente la vista de archivos de ese manifest durante esta consulta; los metadatos de versión/dependencias/features se contrastaron en la ficha y documentación de la misma versión, más la fuente 0.25.1 sólo para confirmar la estructura anterior de features. Si se reconsidera la dependencia, verificar el manifest 0.25.2 directamente antes de fijar la configuración.
- [Código fuente publicado de las funciones de efectos 0.25.2](https://docs.rs/crate/tachyonfx/0.25.2/source/src/fx/mod.rs): efectos de color/timers y modelo de aplicación sobre celdas ya renderizadas.
- Evidencia local: `Cargo.toml` y `Cargo.lock` (Ratatui 0.30.2 / core 0.1.2); `crates/tipsy-ui/src/render.rs::draw_core` (Canvas actual); `crates/tipsy-assistant/src/client.rs::frame_loop` (delta terminal); `crates/tipsy-web/src/lib.rs` (delta RAF); auditoría [VA-01](voice-avatar-audit.md) y contrato [VA-02](voice-avatar-contract.md) (señal y única transición).

La fecha, publicación y licencia son evidencia de metadatos del paquete, no una evaluación legal. No se probó el crate en el repositorio, un navegador ni una terminal. La recomendación nativa queda abierta a revisión visual, pero no deja una cuestión técnica indefinida para implementar VA-04.
