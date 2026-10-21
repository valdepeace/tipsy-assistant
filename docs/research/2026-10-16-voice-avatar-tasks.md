# Tareas para Luna: voz y avatar central

Fecha: 2026-10-16
Estado: backlog preparado; ninguna tarea ejecutada.
Plan: [voz y avatar](2026-10-16-voice-avatar-plan.md).
Spec: [componentes Ratatui](2026-10-16-ratatui-components-spec.md).

## Instrucción de producto obligatoria

El núcleo del centro ES el avatar. La función principal es hablar y recibir respuesta hablada. Chat, métricas y osciloscopio son secundarios. No ensanchar el chat a costa del avatar. No sustituir el núcleo por una biblioteca ni rehacer la arquitectura de audio. Optimizar para conversación no significa prometer un porcentaje de rendimiento.

## Cómo ejecutar este backlog

Asignar a Luna una tarea por turno, usando el prompt del final y el ID correspondiente. Ejecutarlas en orden. No iniciar tareas con dependencias pendientes. No delegar automáticamente ni crear commits/worktrees. Trabajar en el checkout actual y respetar cambios preexistentes.

Cada tarea debe terminar con un informe: resultado, archivos modificados, comprobaciones ejecutadas y resultado, pendientes y estado (`hecha`, `parcial` o `bloqueada`). Registrar el avance en `docs/research/voice-avatar-progress.md`, con una fila por tarea. No marcar como hecho trabajo manual no realizado.

Los nombres de archivos de entrada son puntos de partida verificados al preparar el backlog, no una garantía de que el checkout futuro siga igual. Leer instrucciones locales, comprobar `git status` y localizar símbolos antes de editar. Preferir el grafo MCP; si no está disponible, declarar el fallback y buscar de forma dirigida.

### Límites comunes

- No leer ni imprimir valores de claves; no incluir secretos en evidencias, capturas o logs.
- No hacer llamadas de pago sin autorización explícita. Las pruebas automatizadas deben usar fixtures o mocks.
- No añadir proveedores, wake word, Markdown, textarea, scroll, métricas nuevas, temas ni fondos animados.
- No cambiar el protocolo salvo tarea nueva, acotada y revisada a partir de evidencia.
- Conservar reglas compartidas de `tipsy-ui/src/policy.rs`; no duplicarlas en los clientes.
- No introducir dispositivo, socket, sistema de archivos o reloj del sistema en el renderizador compartido.
- Toda animación recibe tiempo del ciclo de presentación; no usar sleeps ni bloquear captura/reproducción.
- Mantener controles de voz, errores y llamada activa visibles. Respetar español/inglés del perfil.

### Build y validación comunes

La app carga `.env` al arrancar; `cargo run` debe ejecutarse desde la raíz del repo. Cargo toma el directorio de compilación de `.cargo/config.toml`, no del `.env`. No copiar claves a scripts de prueba. Usar `-j 1` en esta máquina; no iniciar una sesión real que pueda generar llamadas de pago durante la validación.

Comandos de referencia, después de preparar el entorno:

```powershell
cargo test -p tipsy-ui -j 1
cargo test -p tipsy-assistant -j 1
cargo test -p tipsy-web -j 1
cargo check -p tipsy-assistant -j 1
cargo check -p tipsy-web --target wasm32-unknown-unknown -j 1
```

Ejecutar solo los pertinentes a los archivos modificados; los gates completos se concentran en VA-08. Comprobar herramientas y target instalados antes; no instalar nada silenciosamente. `cargo run -p xtask -j 1 -- dist` puede descargar recursos y generar el bundle: revisar su flujo y permisos antes de usarlo. Las capturas estáticas no prueban animación, audio ni interacción.

## Dependencias y entregables

| ID | Tipo | Depende de | Entregable principal |
| --- | --- | --- | --- |
| VA-01 | Investigación | — | Auditoría con fuentes y huecos |
| VA-02 | Diseño | VA-01 | Contrato visual completo |
| VA-03 | Investigación | VA-02 | Decisión técnica sobre efectos |
| VA-04 | Código y tests | VA-02 | Representación visual coherente de señales verificadas |
| VA-05 | Código y tests | VA-03, VA-04 | Secuencia sintética y transición A/B |
| VA-06 | Validación/decisión | VA-05 | Comparación visual y elección |
| VA-07 | Código y tests | VA-06 | Layout con avatar protagonista |
| VA-08 | Validación | VA-07 | Informe final y límites comprobados |

Si una dependencia queda bloqueada, no improvisar su resultado. Se pueden documentar hallazgos independientes, pero no ejecutar implementación dependiente.

## VA-01 — Auditar el recorrido de voz y las señales del avatar

**Objetivo:** saber qué ocurre realmente desde el gesto de hablar hasta terminar la respuesta, y qué sabe cada cliente en cada instante.

**Archivos de entrada:** `crates/tipsy-protocol/src/lib.rs`; `crates/tipsy-assistant/src/session.rs`, `brain.rs`, `client.rs`, `client/`, `audio/`, `realtime/`; `crates/tipsy-ui/src/state.rs`, `policy.rs`, `render.rs`; `crates/tipsy-web/src/lib.rs`, `hud.rs`, `mic.rs`, `playback.rs`, `web_audio.rs`.

**Pasos:**

1. Localizar comandos de captura/cancelación, eventos de estado/audio y consumidores en ambos clientes.
2. Trazar por separado: captura nativa, captura navegador, respuesta convencional y llamada en tiempo real. Marcar explícitamente rutas no implementadas.
3. Rastrear escrituras de `status`, `mic_open`, `mic_levels`, `voice_envelope`, `voice_pos_ms`; identificar señal disponible, origen y condición de limpieza.
4. Identificar cómo se reconoce reproducción pendiente, activa y terminada. Distinguir audio recibido, audio en cola y reproducción local; no afirmar que es audible solo por software.
5. Revisar silencio, micro denegado/no disponible, cancelación, interrupción, error de proveedor, desconexión y final de llamada.
6. Localizar controles reales de teclado/ratón para cada acción; no inventar atajos. Revisar pruebas existentes relevantes sin ejecutar proveedores.

**Escritura permitida:** `docs/research/voice-avatar-audit.md` y progreso. No modificar código, manifiestos o lockfile.

**Entregable:** tabla `ruta | etapa | señal y productor | consumidor | estado mostrado | control | condición de salida | evidencia archivo:símbolo/línea | tipo de evidencia | hueco`. Etiquetar evidencia como inspección, prueba automatizada o observación manual. Añadir hallazgos ordenados por impacto.

**Aceptación:** cada ruta tiene inicio/final; toda señal tiene productor/consumidor; los huecos quedan explícitos. Existe una lista concreta de señales utilizables para diseño, sin presentar hipótesis como hechos.

**Parada:** si falta una señal necesaria para representar captura o reproducción, describir el mínimo trabajo requerido y sus consumidores. No corregir el backend dentro de esta tarea.

## VA-02 — Especificar estados y transiciones del avatar

**Objetivo:** convertir la auditoría en reglas visuales implementables.

**Entradas:** auditoría VA-01, `UiState`, `AssistantState`, `draw_core`, política de reproducción y textos ES/EN existentes.

**Pasos:**

1. Definir reposo, escucha, transcripción, pensamiento, habla y fallo. Fallo puede ser un indicador adicional: no añadirlo al enum de protocolo por comodidad.
2. Para cada estado definir señal exacta, entrada/salida, texto ES/EN, geometría/movimiento y reacción al nivel de audio.
3. Resolver con una tabla todos los solapamientos observados: captura y reproducción simultáneas, servidor Idle con audio todavía local, audio en cola sin reproducción, cancelación, reconexión y error.
4. Mantener visible cada hecho relevante aunque haya un estado principal: la llamada activa y el micrófono no pueden quedar escondidos por una animación.
5. Definir movimiento reducido y duración de la transición candidata. Usar una duración concreta como hipótesis de diseño revisable, no como resultado medido.
6. Proponer exactamente una transición inicial localizada en el avatar. No añadir configuraciones de usuario en esta tarea.

**Escritura permitida:** `docs/research/voice-avatar-contract.md` y progreso.

**Aceptación:** cada regla se puede convertir en un test con entradas/salida; no quedan prioridades “a criterio del implementador”. No exigir una señal que VA-01 no demuestre. Registrar por qué la transición mejora la comprensión del estado.

**Parada:** si la evidencia no permite decidir una regla, marcarla pendiente y precisar qué hay que comprobar. VA-04 no empieza con esa regla indefinida.

## VA-03 — Evaluar efectos nativos y TachyonFX sin integrarlos

**Objetivo:** decidir si la transición de VA-02 necesita dependencia externa.

**Entradas:** contrato, manifiestos del workspace/UI/web y `Cargo.lock`. Fuentes primarias: repositorio, manifiesto publicado y documentación de TachyonFX.

**Pasos:**

1. Describir cómo se implementaría la misma transición con Canvas/estilos y tiempo ya disponibles.
2. Localizar una versión publicada concreta de TachyonFX; revisar licencia, `ratatui-core`, funcionalidades por defecto, WASM y temporización.
3. Comparar puntos de integración, dependencias adicionales y necesidad de conservar estado entre frames. No copiar ejemplos con su propio event loop.
4. Proponer dónde viviría ese estado de presentación en ambos clientes, preservando el renderer determinista.
5. Recomendar `solo nativo` o `A/B con TachyonFX`, con versión/features exactos si corresponde. Desactivar DSL si no se utiliza.

**Escritura permitida:** `docs/research/voice-avatar-effects-decision.md` y progreso. Sin instalación ni cambios de manifiestos.

**Aceptación:** alternativa nativa y candidata hacen el mismo efecto; fuentes con fecha; incompatibilidades e incógnitas explícitas. Una rama de desarrollo no se presenta como versión publicada.

**Parada:** incompatibilidad o ventaja poco clara → recomendar solución nativa; no forzar TachyonFX.

## VA-04 — Implementar las reglas visuales verificadas

**Objetivo:** que el avatar represente las señales reales según VA-02, antes de añadir efectos.

**Archivos candidatos:** `tipsy-ui/src/state.rs`, `render.rs`, `strings.rs`, `policy.rs` solo si corresponde; consumidores concretos en cliente nativo y web identificados por VA-01; pruebas existentes próximas.

**Pasos:**

1. Revisar todos los consumidores de las funciones/valores que se tocarán.
2. Reutilizar estado existente; añadir únicamente un valor de presentación derivado si evita duplicación real. No crear un framework de estados.
3. Aplicar reglas de VA-02 a núcleo y etiqueta accesible, conservando su identidad y áreas de interacción.
4. Cuando la señal ya existe localmente pero no llega a UI, conectar productor/consumidor en ambos clientes con el mínimo cambio. No cambiar captura, colas, generación o protocolo.
5. Añadir tests significativos para solapamientos, cancelación y fin de reproducción. El test debe comprobar la regla, no un detalle interno arbitrario.

**Escritura permitida:** archivos anteriores realmente necesarios y tests; anotar el alcance exacto antes de editar. No dependencias nuevas.

**Aceptación:** tabla VA-02 implementada; tests de estados/solapamientos pasan; no se finge escucha o habla ante señales insuficientes; UI compartida conserva compilación WASM. No se altera la política de cancelación.

**Parada:** si exige arreglar audio o cambiar protocolo, documentar una tarea nueva con bug, reproducción y alcance, y detener esa parte. No expandir VA-04.

## VA-05 — Construir comparación determinista de una transición

**Objetivo:** comparar visualmente el avatar de VA-04 con y sin la transición elegida.

**Entradas:** contrato VA-02 y decisión VA-03. Archivos candidatos: `xtask/src/shots.rs`, renderer y ciclos de presentación nativo/web identificados en la auditoría.

**Pasos:**

1. Definir una única secuencia de datos sintéticos y tiempos explícitos: reposo → escucha silenciosa → voz → transcripción → pensamiento → habla → reposo. Añadir escenarios separados de cancelación y fallo.
2. Reutilizar mecanismos de fixtures/capturas existentes; añadir el mínimo punto de entrada de demostración solo si no existe uno apto. Documentar cómo activarlo; no iniciar proveedores, sockets o dispositivos reales.
3. A = avatar sin transición nueva. B = mismo estado, tamaño y tiempos con transición. Si VA-03 eligió nativo, B usa esa solución; si eligió TachyonFX, usar versión/features definidos.
4. Aplicar efecto solo a la región acordada. Avanzarlo con delta de tiempo del loop existente y conservar estado entre frames cuando corresponda.
5. Producir capturas de puntos temporales fijos y facilitar la inspección animada en terminal y navegador. No crear una infraestructura de preview independiente.
6. Limitar movimiento reducido a la ruta de presentación mínima definida en el contrato; no añadir una pantalla de opciones nueva.

**Escritura permitida:** archivos de presentación/fixtures necesarios, manifiestos y lockfile solo si VA-03 admite TachyonFX. Crear `docs/research/voice-avatar-preview.md` con comandos reales de reproducción y etiquetas SINTÉTICO.

**Aceptación:** A/B reproduce exactamente las mismas señales y tiempos; ningún proveedor se ejecuta; capturas repetibles; construcción nativa/WASM pertinente pasa; camino animado reproducible documentado. No atribuir audio real a este experimento.

**Parada:** si preview web obliga a reescribir bootstrap o servidor, describir la limitación y proponer un alcance menor antes de seguir.

## VA-06 — Inspeccionar A/B y elegir la solución

**Objetivo:** elegir por evidencia de comprensión visual y comportamiento, no por catálogo.

**Pasos:**

1. Ejecutar la secuencia VA-05 en terminal y navegador con mismos estados y dimensiones.
2. Observar identificación de estados, legibilidad, cancelación visual, conservación de avisos, interacción y fluidez. No inventar una observación visual si no se dispone de interfaz.
3. Comprobar movimiento reducido. Registrar captura o evidencia reproducible y entorno.
4. Elegir A o B justificando la aportación. Si no hay ventaja clara, elegir A y retirar integración/dependencia experimental innecesaria sin tocar trabajo ajeno.

**Entregable:** `docs/research/voice-avatar-visual-review.md`, con tabla `criterio | terminal | navegador | evidencia | resultado` y decisión final. Cambios de limpieza limitados al experimento propio.

**Aceptación:** decisión explícita y respaldada por inspección; limitaciones declaradas. Una imagen fija no se usa como prueba de animación.

**Parada:** sin acceso visual a una superficie, dejar su revisión pendiente y proporcionar pasos concretos de inspección. No marcar esta tarea hecha ni elegir B como validado en ambas superficies.

## VA-07 — Adaptar layout conservando el avatar protagonista

**Objetivo:** mantener avatar, estado y controles de voz útiles en pantallas de distinto tamaño.

**Entradas:** solución elegida VA-06; `draw_hud`, `draw_header`, layout y `Hit`; renderizadores de configuración/login solo para verificar que no se rompen.

**Pasos:**

1. Capturar composición actual a 120×40, 80×24, 60×20 y 40×15 celdas. Son casos de prueba, no breakpoints automáticos.
2. Proponer mínimos y umbrales a partir del espacio necesario para avatar, estado y controles. Registrarlos antes de implementar; no derivarlos del tamaño del chat.
3. Reutilizar Layout: reducir/reubicar/omitir primero métricas y chat. El osciloscopio permanece secundario debajo del avatar cuando haya espacio.
4. Conservar acceso a entrada de texto y configuración; no ocultar errores, micro o llamada activa. No crear paneles nuevos para compensar espacio.
5. Recalcular `Hit` desde la geometría real. Verificar que una zona invisible no sigue siendo clicable y que un clic no activa otro control.
6. Añadir pruebas de composición y hits para tamaños representativos; actualizar capturas pertinentes.

**Escritura permitida:** renderer compartido y tests/capturas asociados; textos solo si se necesitan etiquetas compactas ES/EN. Documentar decisiones en `voice-avatar-visual-review.md`.

**Aceptación:** el avatar no desaparece para favorecer complementos; no hay solapamiento de estado/controles en casos probados; clicks corresponden a controles visibles; configuración/login conservan su función.

**Parada:** si un tamaño es insuficiente, mostrar una salida mínima honesta con estado/controles y documentar el mínimo soportado. No afirmar soporte pleno sin espacio.

## VA-08 — Validar la entrega completa y registrar límites

**Objetivo:** comprobar la mejora sin confundir simulación con conversación real.

**Pasos automatizados:** ejecutar tests de UI, políticas y clientes afectados; checks nativo y WASM; generación de capturas pertinente; revisión del grafo de dependencias del objetivo WASM; revisión final del diff. Registrar comandos, resultados y fallos preexistentes separados de regresiones. No usar el árbol nativo completo para concluir que Crossterm contamina WASM.

**Pasos manuales, después de disponer de dispositivos y autorización para cualquier consumo:**

1. En terminal y navegador: iniciar captura, hablar, cerrar turno, recibir/oír respuesta y repetir.
2. Probar silencio, micro denegado/no disponible y un fallo recuperable controlado.
3. Probar cancelación/interrupción con los controles reales documentados en VA-01; final de reproducción y llamada si esa ruta existe y está autorizada.
4. Redimensionar durante escucha y reproducción. Confirmar estado, controles y protagonismo del avatar.
5. Comparar antes/después bajo condiciones equivalentes. Registrar fin de captura, primer audio recibido y comienzo local de reproducción solo si las señales permiten medirlos. No presentar una marca software como medición acústica.

**Entregable:** `docs/research/voice-avatar-validation.md` con `caso | superficie/modo | precondición | pasos | esperado | observado | evidencia | pasa/falla/no ejecutado`; resumen de cambios, limitaciones y limpieza del experimento. Actualizar progreso.

**Aceptación:** checks pertinentes pasan; ambos clientes inspeccionados; no queda avatar hablando tras cancelación/fin; errores recuperables legibles; audio y controles no presentan regresiones observadas. Las pruebas reales no ejecutadas impiden declarar validación completa.

**Parada:** ausencia de permisos/dispositivos/autorización → no ejecutar llamadas y dejar casos pendientes con instrucciones para el usuario. No sustituirlos por mocks y marcar pasa.

## Prompt listo para asignar a Luna

```text
Ejecuta únicamente la tarea VA-XX de docs/research/2026-10-16-voice-avatar-tasks.md.
Lee primero las reglas comunes, la spec, el plan y los entregables de sus dependencias.
Comprueba que las dependencias están terminadas; no inventes su resultado.
El núcleo central es nuestro avatar: la conversación por voz manda y los demás paneles son secundarios.
Trabaja en el checkout actual, preserva cambios previos y respeta el alcance de archivos y las condiciones de parada.
No hagas llamadas de pago ni instales componentes fuera de lo autorizado por la tarea.
Antes de editar indica el alcance concreto. Implementa o investiga según el tipo de tarea y ejecuta las comprobaciones pertinentes.
Actualiza docs/research/voice-avatar-progress.md y entrega: resultado, archivos, evidencia, comprobaciones, pendientes y estado.
No continúes automáticamente con otra tarea y no hagas commits.
```

## Cierre global

El backlog termina cuando el avatar comunica bien los estados de conversación reales, mantiene protagonismo en tamaños soportados y no hay regresiones observadas de voz/controles en las superficies verificadas. Añadir componentes visuales por variedad no es un criterio de éxito.
