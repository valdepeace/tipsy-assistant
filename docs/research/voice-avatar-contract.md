# Contrato visual del avatar — VA-02

Fecha: 2026-10-16  
Estado: contrato implementado parcialmente por VA-04; pendiente de inspección visual VA-06 y validación global VA-08.  
Evidencia base: [auditoría VA-01](voice-avatar-audit.md), en particular «Recorrido y evidencia», «Señales verificadas utilizables para diseño», «Hallazgos por impacto» y «Huecos y condición de parada».

## Límite del contrato

El centro del HUD sigue siendo el avatar existente (`tipsy-ui/src/render.rs:draw_rings/draw_core`). El núcleo representa el estado principal; texto comprensible acompaña el color y la forma. La onda queda debajo y secundaria. No se añaden estados al protocolo, una pantalla de opciones ni controles/hits nuevos.

La auditoría es inspección estática: no demuestra captura con hardware, que un altavoz produjo sonido, la apariencia en vivo ni el comportamiento acústico. En particular, `Player::playing()` y `Playback::is_playing(now)` describen salida que el cliente mantiene en su reproductor/timeline; no certifican que una persona la oyera. El navegador tampoco separa con su `is_playing` un nodo programado para el futuro de uno ya iniciado: `Live` solo guarda `ends_at`. Para evitar afirmaciones falsas, el contrato agrupa ese caso como **salida local en cola/programada o en curso**, con esa misma precisión en el texto.

## Entradas exactas

El consumidor arma esta instantánea en cada presentación, sin consultar el reloj, dispositivos o sockets desde el renderer compartido:

| dato | fuente autorizada | significado en este contrato |
|---|---|---|
| `status` | último `Event::Status` aplicado a `UiState` | Última actividad informada por el servidor: `Idle`, `Listening`, `Transcribing`, `Thinking` o `Speaking`. No describe por sí sola la salida local. Auditoría: `state.rs:apply` y `render.rs:header`. |
| `live` | último `Event::Live` aplicado a `UiState` | Llamada Realtime nativa viva; solo este evento la eleva o baja. Auditoría: fila «Nativa, Realtime» y `client/call.rs:call_ended`. |
| `mic_enabled` | `UiState.mic_open`; durante llamada nativa, además `live` | Captura solicitada/habilitada por HUD. Nativo es el switch que puede fallar al abrir el dispositivo; web se activa tras el gate y controla qué muestras se envían. No equivale a voz detectada ni es una señal común de stream físico. Auditoría: filas «iniciar captura» y «gesto inicial y permiso». |
| `local_output` | nativo: `Client::playing()` (`Player::playing`); web: `Playback::is_playing(audio_context.current_time())` | El reproductor/timeline local conserva audio pendiente o programado. En web puede incluir nodos futuros (`Live` solo guarda `ends_at`). El fin se observa por `Player::playing()==false` o por `Playback::finished(now)`/`is_playing(now)==false`. No es prueba acústica. Auditoría: filas «respuesta y reproducción» y señal homónima. |
| `native_progress` (evidencia secundaria; no activa estado) | `Player::progress()` | En llamada nativa informa fuentes restantes y posición del reproductor. La auditoría solo encontró su uso para truncar Realtime; no valida un mapa de chunks común ni su paso al avatar. No deducir de aquí un playhead/porcentaje visual en VA-02. |
| `reply_error` | `LogEntry.error.is_some()` de la última entrada assistant | Indicador compacto del fallo más reciente; el texto del error sigue en su entrada del log. Errores antiguos conservados en el historial no enmascaran respuestas posteriores. Auditoría: fila «fallo de micro, proveedor o conexión» y `state.rs:apply(Event::Error/Reply)`. |
| `notice` | `UiState.notice` | Texto temporal existente. No clasificarlo como fallo ni parsear su contenido: se usa también para avisos no fatales y no lleva tipo. Se conserva visible con su duración actual. |
| `mic_levels`, `voice_envelope`, `voice_pos_ms` | niveles RMS capturados, PCM recibido y `UiState::tick` | No determinan estado, color, movimiento ni pulso del avatar. La onda secundaria sigue usando estos datos, pero los niveles no llevan frescura y `voice_pos_ms` no sigue la posición real de reproducción. Auditoría: «Señales verificadas». |

No hay señal/configuración de movimiento reducido en las superficies inspeccionadas (`tipsy-ui`, `tipsy-web` y cliente nativo). No se añade un toggle ni una preferencia nueva.

## Estado principal y prioridad

Evaluar en este orden; la primera condición satisfecha fija el rótulo y la forma central. Las banderas de llamada, micrófono y avisos se pintan en capas independientes y no se esconden por el estado principal.

| prioridad | condición exacta | estado / texto ES — EN | entrada y salida | forma/movimiento del núcleo |
|---:|---|---|---|---|
| 1 | `reply_error` | **Error en la respuesta — Reply failed**. Mantener además el mensaje original visible en el log. | Entra cuando se marca `LogEntry.error`; sale cuando una respuesta válida limpia ese campo o al borrar el historial. | Anillo abierto con marca `!`, quieto. No pulsar ni ocultar el error con movimiento. |
| 2 | `local_output` | **Salida de voz local en cola o en curso — Local voice queued or in progress**. | Entra al ser aceptado/encolado audio por la salida local; continúa aunque `status` pase a `Idle`; sale solo cuando el reproductor/timeline deja de reportar salida local. | Anillo central abierto y pulso suave, sin modular amplitud con `voice_envelope`. El texto no dice “audible”. |
| 3 | `live && status == Idle` | **Llamada en vivo, esperando estado — Live call, waiting for state**. | Entra si la llamada permanece viva pero el último estado es Idle; sale con otro `status` o `Live(false)`. | Anillo doble estable y micrófono visible; no simular escucha activa. |
| 4 | `live && status == Listening` | **Escucha en vivo — Live listening**. Mantener `EN VIVO — LIVE` como distintivo independiente durante toda la llamada. | Entra con `live` y `Listening`; cambia con `status` o `live`. | Anillo doble estable con micrófono visible; sin pulso basado en `mic_levels`. |
| 5 | `mic_enabled && (status == Listening || status == Idle)` | **Micrófono habilitado — Microphone enabled**. | Entra al habilitar el switch/gate; sale cuando se apaga. Si abrir el dispositivo falla, se apaga y queda el aviso. | Anillo rojo estable y glyph de micrófono; sin reacción al nivel histórico ni a silencio. |
| 6 | `status == Transcribing` | **Transcribiendo… — Transcribing…** | Entra/sale con `Status(Transcribing)` y siguiente status/error. Puede coexistir con `mic_enabled` (p. ej. VAD auto); conservar ese distintivo. | Anillo segmentado, movimiento lento e indeterminado, sin porcentaje. |
| 7 | `status == Thinking` | **Pensando… — Thinking…** | Entra/sale con eventos de estado; no inferir progreso numérico. | Arcos existentes en rotación ocupada, sin pulso de voz. |
| 8 | `live && status == Speaking && !local_output` | **Respuesta Realtime del servidor — Realtime server response**. | Entra cuando el estado de llamada dice `Speaking` sin salida local reportada; sale al detectar salida o al cambiar status/live. No equivale a habla ya reproducida. | Núcleo estático con marcas de espera; distintivo `LIVE` fijo; sin onda/pulso de reproducción. |
| 9 | `status == Speaking && !local_output` (no cubierto arriba) | **Esperando salida de voz local — Waiting for local voice output**. | Entra cuando el servidor anuncia `Speaking` sin salida local; sale al detectarla o con nuevo status/error. | Centro estático con tres marcas cortas hacia fuera; no onda ni pulso que parezca reproducción. |
| 10 | `status == Listening && !mic_enabled && !live` | **Esperando micrófono — Waiting for microphone**. | Entra por estado servidor Listening sin petición local ni llamada; sale al cambiar cualquiera de esas señales. | Anillo discontinuo y glyph de micrófono apagado; quieto. |
| 11 | resto, incluido `status == Idle` sin salida local, llamada, micrófono habilitado ni fallo de respuesta | **Listo, quillo — Ready** (reutilizar strings actuales). | Estado base; se reemplaza en cuanto entra una condición de mayor prioridad. | Geometría actual, anillo cerrado y movimiento ambiental lento. |

`notice` no es una décima condición ni cambia por sí solo el estado principal: es un aviso independiente visible durante la duración que le da hoy `UiState`; el contrato no prolonga avisos temporales. Esto evita convertir `Conexión restablecida`, `Conversación borrada` o un aviso de modo no disponible en un falso estado de fallo. Ninguna animación puede adelantar su desaparición. `reply_error`, a diferencia del aviso, permanece según la limpieza descrita en la primera fila.

## Resolución explícita de solapamientos

| señales simultáneas | estado principal | capas que permanecen visibles | motivo / límite |
|---|---|---|---|
| `reply_error` + cualquier `status`, salida local, micrófono o llamada | Error en la respuesta de la entrada assistant más reciente | mensaje en el log; `EN VIVO` si `live`; micrófono si `mic_enabled` o `live`; insignia de salida local si sigue presente; aviso temporal si lo hay | El fallo tiene prioridad textual. La animación nunca borra el hecho de llamada/captura/salida. Errores más antiguos siguen visibles en el historial, no como estado actual. |
| salida local + `status == Idle` tras `Done`/`Idle` | Salida de voz local en cola o en curso | `mic_enabled`/`live` y avisos, si los hay | `Done`/`Idle` cierra el turno servidor, no la cola local. El estado dura hasta el fin local reportado. |
| salida local + `Listening`, `Transcribing`, `Thinking` o `Speaking` | Salida de voz local en cola o en curso | distintivo de micrófono/llamada y etiqueta secundaria `Servidor: {estado} — Server: {state}` para cualquier estado distinto de `Idle` | No ocultar una captura/llamada simultánea ni el último estado del servidor. La etiqueta secundaria conserva ese hecho sin degradar salida local a PCM meramente recibido. |
| PCM recibido pero `local_output == false` | Aplicar reglas de `status`; si `Speaking`, queda «Esperando salida de voz local» | micrófono, llamada y aviso/error | El evento o las muestras recibidas no son reproducción. Si el navegador aún no creó `AudioContext`, los chunks se descartan y el gate/aviso explica la ruta sin salida. |
| `mic_enabled` + silencio (ningún nivel nuevo o RMS bajo) | Micrófono habilitado si `status` es `Idle`/`Listening`; en otro caso manda el estado de servidor | distintivo de micrófono siempre | No declarar silencio medido ni reacción de escucha a partir de muestras antiguas. No existe timestamp fresco común para RMS. |
| captura habilitada + `Transcribing` en modo auto | Transcribiendo… | distintivo rojo de micrófono habilitado | El servidor puede transcribir una frase mientras el cliente mantiene el stream para el siguiente turno. |
| llamada activa + estado de pensamiento/transcripción/speaking | Estado determinado por tabla de prioridad | `EN VIVO — LIVE` fijo y micrófono fijo derivados de `live` | `live` significa que la llamada sigue abierta, no que el asistente hable. |
| llamada activa + `status == Idle` | Llamada en vivo, esperando estado | `EN VIVO — LIVE` y micrófono | Evita etiquetar como lista o escuchando una llamada que aún no informa escucha. |
| llamada activa + `status == Speaking` pero `local_output == false` | Respuesta Realtime del servidor | `EN VIVO — LIVE` y micrófono | `Speaking` solo prueba el estado del servidor; no prueba inicio de reproducción local. |
| llamada activa + `Tab` barge-in + `local_output` ya falso, pero aún no llega `Listening` | El último estado server según la tabla (si aún es `Speaking`, «Esperando salida de voz local») | `EN VIVO — LIVE` y micrófono | El cliente silencia antes del siguiente evento del servidor. No hay marca compartida que diga “Tab interrumpió”; no inventar un estado Barge-in ni deducirlo de RMS. Al llegar `Listening`, cambia a escucha en vivo. |
| `Tab`/`F3` cierra una captura convencional mientras salía voz | Salida local mientras `local_output` siga true; después `Transcribing` con su micrófono ya apagado | Aviso/error, si aplica | `ListenStop` cierra turno y la política silencia salida que aún esté en cola. No equivale a descartar el audio capturado. |
| `Cancel` convencional, con/sin audio local | Mientras `local_output` siga true, salida local; al caer, estado server resultante (`Idle` o Listening si el micrófono quedó habilitado) | Aviso/error; micrófono si sigue habilitado | `Cancel` produce `Idle`, pero no hay tecla Cancel dedicada. No cambiar el control ni la política desde el contrato. |
| `reply_error` asociado a turno ya Idle y sin salida local | Error en la respuesta | error persistente en el log | Error permanece tras `Idle`; una respuesta válida o limpiar conversación lo elimina. |
| error de apertura/permiso de micrófono, sin `reply_error` | estado base derivado de señales (`Ready` o `Waiting for microphone`) | aviso existente con motivo exacto, siempre encima/visible | `notice` no está tipado y no puede activar el estado `Error` sin clasificar avisos inocuos como fallos. El switch queda apagado; no afirmar escucha. |
| pérdida/reconexión de socket, sin `reply_error` | último estado derivable de señales; nunca declarar nueva captura ni salida | aviso de conexión existente visible; mantener `live` solo mientras siga `true` | `UiState` no expone un booleano de conexión común ni una clase tipada para cada `notice`. No deducir “reconectando” parseando texto localizado, ni borrar el aviso con animación. `Welcome`/estado nuevo actualizará el estado cuando llegue. |
| navegador recibe F10 / Realtime no implementado | estado normal determinado por `status`/`mic_enabled` | aviso `live_web_unavailable` visible; no mostrar `LIVE` | El navegador no crea llamada, `live` sigue falso. |

## Texto y jerarquía visual

Usar el idioma del perfil (`UiState.lang` → `tipsy_ui::strings`), nunca cadenas elegidas por el cliente. Las etiquetas actuales `ready`, `transcribing`, `thinking`, `speaking`, `listening` y `live` están en ambas tablas (`tipsy-ui/src/strings.rs`, `ES` y `EN`). Para los estados derivados, añadir estas cadenas a ambas tablas:

| clave propuesta | ES | EN |
|---|---|---|
| `mic_enabled` | Micrófono habilitado | Microphone enabled |
| `waiting_mic` | Esperando micrófono | Waiting for microphone |
| `output_pending` | Esperando salida de voz local | Waiting for local voice output |
| `output_local` | Salida de voz local en cola o en curso | Local voice queued or in progress |
| `reply_failed` | Error en la respuesta | Reply failed |
| `server_status` | Servidor: {estado} | Server: {state} |
| `live_listening` | Escucha en vivo | Live listening |
| `live_waiting_status` | Llamada en vivo, esperando estado | Live call, waiting for state |
| `live_responding` | Respuesta Realtime del servidor | Realtime server response |

La forma del núcleo y el rótulo comunican estado sin depender de color. El aviso existente y el texto del error permanecen legibles. El avatar conserva el área del `draw_core`; la onda sigue en el panel de voz secundario. La transición no mueve paneles ni controles.

Todos los estados ignoran `mic_levels`, `voice_envelope` y `voice_pos_ms` al decidir movimiento o intensidad del núcleo. No hay timestamp/frescura común para los niveles de captura ni posición de salida conectada al avatar. La onda existente continúa como visualización secundaria; no alimenta el contrato del núcleo.

## Única transición inicial

Propuesta: al cambiar `mic_enabled` de falso a verdadero, el anillo central interpola de cian a rojo durante **180 ms**; sale inmediatamente si la señal vuelve a falso por cierre o fallo. El texto y el glyph de micrófono aparecen desde el primer frame. La transición ocurre exclusivamente dentro de `draw_core`, comunica que el micrófono quedó habilitado/solicitado y no anima una onda ni afirma que llegó voz. Los 180 ms son hipótesis de diseño, no una duración medida.

Movimiento reducido: no hay ajuste ni señal de sistema visible en las rutas actuales; no añadir opción. Las formas y textos anteriores son suficientes sin animación; la transición es solo refuerzo, no fuente única de significado. El estado final se puede dibujar inmediatamente si el host incorpora en el futuro una señal de movimiento reducido.

## Regiones e interacción

- Estado y avisos siguen en la línea de estado del header; `live` y el micrófono tienen distintivos persistentes separados. Error asociado a respuesta conserva su texto en el log.
- La forma y la única transición se limitan al rectángulo que ya recibe `draw_core` (`draw_rings` dentro del panel de núcleo). El osciloscopio queda debajo y no controla el estado principal.
- No añadir ni mover `Hit`. La auditoría no encontró hit de ratón para el micrófono: conserva `Tab`/`F3`; `F10` solo en nativo; la puerta HTML existente habilita captura web. El engrane/modo y borrado mantienen sus áreas; F2 mantiene reproducción seleccionada.
- Ningún indicador desaparece por rotación/pulso: `EN VIVO`, micrófono y aviso/error no son parte animada del anillo.

## Condiciones aún no demostradas

1. **Audibilidad física:** las dos señales locales solo demuestran salida en cola/programada o en curso en el software. Si producto exige diferenciar “pendiente”, “reproducción iniciada” y “terminada”, el mínimo futuro es exponer inicio/posición del reproductor en ambas presentaciones. El contrato no llama «audible» a ninguna de ellas.
2. **Barge-in de llamada:** el renderer compartido no recibe la acción local `Tab` que manda `response.cancel`. La regla segura es mantener `LIVE`, mostrar salida local solo mientras exista y esperar el siguiente `Status`; añadir una marca visual de interrupción requeriría una señal de presentación derivada del cliente, sin protocolo.
3. **Error de micrófono y reconexión:** algunos caminos llegan solo como `notice` sin tipo. Mantener el aviso existente por encima del avatar; no clasificarlos en el estado de fallo hasta que la UI disponga de una clase fiable. Los avisos y el estado exacto de cada superficie no se observaron manualmente.
4. **Movimiento reducido:** sin preferencia de sistema/configuración común, solo se especifica que el significado no depende del movimiento. No se define un control de usuario.

Estas limitaciones no impiden implementar la jerarquía agrupada en VA-04; sí impiden prometer una señal acústica, distinguir cola física de reproducción real o dibujar un estado específico de “reconectando”/“barge-in” sin ampliar las señales locales.

