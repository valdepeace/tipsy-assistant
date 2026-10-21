# Plan: ventana flotante del núcleo

Fecha: 2026-10-21  
Estado: spike de ventana compilado y abierto; inspección manual y audio pendientes.  
Spec: [ventana flotante del núcleo](2026-10-21-floating-core-window-spec.md).

## Resultado

Añadir un modo de escritorio opcional para Windows que muestre el núcleo en una ventana pequeña, siempre encima de ventanas normales, y permita seguir interactuando por voz. El TUI actual, el servidor, el cliente web y el terminal web siguen disponibles y conservan el comportamiento actual.

La propuesta inicial es un comando nuevo `tipsy-assistant window`, una sesión independiente y una sola ventana que cambia entre vista completa y compacta. El comando por defecto no cambia.

## Base observada en el código

- `tipsy-assistant` sin subcomando arranca el TUI nativo; `--connect` es otro flujo existente.
- `serve` prepara configuración y proveedores y ejecuta Axum. El servidor sirve el cliente WebAssembly desde el bundle generado por `xtask dist`.
- `tipsy-web` deriva el WebSocket de `window.location` y el servidor comprueba el origen. La vista debe abrirse bajo el origen local servido por Tipsy si se reutiliza ese cliente.
- La vista WebAssembly usa WebSocket, WebAudio, `getUserMedia` y el canvas WebGL2 de Ratzilla.
- El TUI y el cliente web reutilizan `tipsy-ui`, pero tienen backends y rutas de audio distintas.
- `portable-pty` ya soporta ConPTY para el terminal web; no crea una ventana gráfica ni renderiza el HUD.

## Principios de implementación

1. El modo flotante es opt-in y no toma el lugar de ningún comando actual.
2. Una sesión por ejecución del modo ventana. Cambiar foco o tamaño solo cambia presentación; no crea clientes ni sesiones nuevas.
3. Cerrar el modo ventana cierra su servidor loopback y recursos propios. No mata ni modifica otro proceso TUI o `serve`.
4. Reutilizar primero `tipsy-web` y el HUD existente dentro de WebView2. Solo reconsiderar un frontend nativo si una limitación comprobada lo exige.
5. No cambiar el protocolo para resolver un requisito puramente de ventana/presentación.
6. No añadir preferencia persistente de tamaño, esquina o topmost en el primer corte.

## Etapa 0 — cerrar decisiones de producto

Confirmar en la spec:

- comando `window` frente a bandera;
- transición al perder/recuperar foco;
- controles disponibles en compacto;
- ubicación inicial y movimiento;
- servidor loopback propio frente a servidor preexistente;
- funcionamiento de una sesión de ventana en paralelo con TUI.

**Salida:** spec aprobada con requisitos y límites explícitos. No añadir código de producción todavía.

## Etapa 1 — prueba de ventana WebView2

Construir un spike mínimo y aislado que abra la ruta del cliente ya servida por Tipsy, usando un origen loopback. Comprobar creación/cierre de ventana, topmost, dimensiones, posición, foco y carga de WASM/WebGL2. Probar permiso y captura de micrófono, audio de salida y conexión al WebSocket.

No implementar aún flujo de configuración, instalador, iconos, preferencias ni integración completa en el CLI.

**Puerta de decisión:** continuar con WebView2 solo si el origen se acepta, el bundle carga, el audio funciona tras gesto/permisos y el cambio de foco no reinicia la página ni la sesión. Si falla, aislar la causa antes de elegir un host distinto.

### Resultado parcial — 2026-10-21

- Spike aislado creado en `poc/floating-window/`; no se añadieron dependencias al workspace ni se cambió el CLI de producción.
- `cargo test --manifest-path poc/floating-window/Cargo.toml -j 1`: 2 comprobaciones pasan. `cargo build` del spike también pasa.
- `cargo run -p xtask -- dist` generó `web/dist/` y compiló el binario anfitrión. La ruta `/` respondió HTTP 200; `tipsy_web.js` y `tipsy_web_bg.wasm` respondieron 200.
- Se inició el spike contra `http://127.0.0.1:7070/`; la ventana actual sigue abierta tras corregir el anclaje de la expansión.
- En esa primera ejecución se detectó y corrigió que maximizar desde la esquina podía dejar la ventana fuera de pantalla; ambos tamaños se anclan ahora por separado a la misma esquina.
- Verificación posterior: el proceso Tauri responde y la página (7410 bytes) y `tipsy_web.js` siguen respondiendo HTTP 200.
- El cliente web acepta `floating=1`: la página deja que el canvas siga el viewport y Ratzilla vuelve a medir la rejilla cuando cambia el tamaño; solo en ese modo y bajo 600 px se dibujan el núcleo y el osciloscopio de voz existente, en vertical. Un clic en la vista compacta usa el atajo existente F3 para activar/parar el micrófono. La web normal y el TUI siguen usando `draw` sin cambios.
- El tamaño compacto del spike quedó en 220×165 (antes 240×180) tras la revisión del usuario.
- El test compacto a 36×20 comprueba que no se pinta el HUD; el test de la página comprueba que el canvas flotante usa el tamaño del viewport.
- El test específico del render compacto y el test del parámetro flotante pasan; `xtask dist` volvió a generar el WASM y el ejecutable que lo sirve. La ventana nueva y el servidor loopback quedan abiertos para inspección.
- La suite combinada de `tipsy-ui` y `tipsy-web` reportó además dos fallos no relacionados con este modo: el texto esperado `LIVE` frente al chip localizado `EN VIVO`, y una longitud esperada 130 frente a 132 en el test de `tone_floor`.
- La ventana visible, su cambio de tamaño/posición/topmost, el clic en el núcleo, el permiso del micrófono y la salida audible quedan pendientes de inspección manual. No declarar la puerta superada hasta observarlos.
- El servidor de prueba está ligado al loopback en el puerto 7070. Dejarlo activo durante la inspección; se inició desde el binario local generado por `dist`.

## Etapa 2 — encajar el ciclo de vida en Tipsy

Añadir una ruta de inicio explícita para la ventana. Reutilizar la carga de configuración, resolución de credenciales, construcción de proveedores y el servidor existentes, evitando copiar la lógica de `serve`. Vincular el listener a loopback y a un puerto efímero; entregar la dirección realmente asignada al WebView. Al cerrar la ventana, detener el servidor iniciado por esa ejecución.

Mantener `run_client`, `run_remote_terminal` y `serve_command` sin cambios de comportamiento. Si compartir helpers requiere reestructuración amplia, parar y documentar antes de hacerla.

**Salida:** el nuevo comando inicia una sesión web local y cierra únicamente sus propios recursos.

## Etapa 3 — ventana compacta e interacción

Implementar una sola ventana con vista completa y compacta. Al perder foco: reducir y ubicar en la esquina acordada, activar topmost y mantener controles accionables. Al recuperar foco: restaurar tamaño completo. La acción de hablar debe cumplir el requisito de gesto explícito del navegador para micrófono/audio.

El avatar conserva prioridad; se esconden primero paneles secundarios. No dejar controles solo por color, fuera de bounds o cubiertos por overlays.

**Salida:** lista revisable de controles y una inspección visual manual de ambas dimensiones.

## Etapa 4 — regresión, seguridad y empaquetado mínimo

- Verificar que arranque por defecto sigue yendo al TUI y que `serve`, `--connect`, web y terminal web conservan rutas.
- Verificar binds loopback, token/origin, cierre ordenado y limpieza de la instancia propia.
- Verificar que un puerto ocupado no hace que la ventana se conecte accidentalmente a otro servicio.
- Comprobar micrófono denegado, proveedor sin clave, fallo de servidor y cierre de WebView con mensajes recuperables.
- Revisar dependencias Windows y el empaquetado mínimo solo después de que el spike pase.

**Aceptación:** todos los criterios de la spec; evidencias automatizadas y manuales etiquetadas por separado.

## Riesgos y límites

- WebView2 puede requerir manejo explícito de permisos de micrófono/audio.
- El diseño actual del HUD web ocupa el viewport; la composición compacta puede requerir un layout dedicado en `tipsy-ui` o una escala/recorte inadecuados. No asumir que reducir la ventana basta.
- `always on top` cubre ventanas normales, no UAC/escritorio seguro ni necesariamente pantalla completa exclusiva.
- Tauri/Wry y la integración con el loop Tokio pueden añadir coste de compilación y plataforma; medirlo en el spike antes de fijar tecnología.
- El cliente web y el TUI en procesos separados no comparten conversación ni dispositivo de audio; ambos pueden competir por el micrófono si se usan simultáneamente.

## Fuera de alcance

No reemplazar el cliente TUI, no migrar todo el producto a escritorio, no construir un emulador de terminal, no sincronizar clientes ni cambiar proveedores/protocolo/audio para una ventaja visual.

## Regla de parada

Parar tras un prototipo de Windows que cumpla la interacción y las regresiones de la spec. No añadir bandeja, autoarranque, transparencia, paso de clics, preferencias persistentes ni soporte multiplataforma sin una petición y una necesidad comprobada.
