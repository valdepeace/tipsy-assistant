# Plan: conversación por voz y avatar central

Fecha: 2026-10-16
Estado: plan para revisión; implementación pendiente.
Spec: [Componentes Ratatui](2026-10-16-ratatui-components-spec.md).

## Resultado esperado y límites

El núcleo central es el avatar de Tipsy. La experiencia se organiza alrededor de hablar y recibir respuesta hablada. Chat, métricas y osciloscopio son secundarios. Mantener el avatar protagonista también en ventanas estrechas.

El plan empieza por reunir evidencia del recorrido actual y termina con una mejora visual pequeña, validada en terminal y navegador. No promete una optimización porcentual. No incluye nuevos proveedores, wake word, cambios de modelo, una reescritura de audio, Markdown ni mejoras del chat.

No añadir dependencias antes de evaluar la solución existente. Las pruebas simuladas no demuestran conversación real. Las llamadas a proveedores de pago requieren autorización explícita antes de ejecutarse.

## Base comprobada al preparar el plan

- `AssistantState` distingue `Idle`, `Listening`, `Transcribing`, `Thinking` y `Speaking` (`crates/tipsy-protocol/src/lib.rs`). El fallo se debe estudiar mediante sus eventos, no asumir un estado adicional en el enum.
- Existen `ListenStart`, `ListenStop` y `Cancel`; comprobar su accesibilidad y efecto real en cada cliente, sin confundir contrato con funcionamiento validado.
- `tipsy-ui/src/policy.rs` comparte decisiones de interrupción de reproducción. Reutilizarlas.
- `tipsy-ui/src/render.rs` contiene `draw_core` y `draw_voice`; el núcleo ya reacciona al tiempo, actividad y nivel de voz.
- `UiState` contiene estado, apertura de micrófono, envolventes y posición de reproducción. Estos datos son candidatos para el avatar; verificar quién los actualiza y cuándo.
- El renderizador recibe estado y tiempo; no consulta dispositivos ni reloj del sistema. Preservar esa separación.
- El manifiesto del workspace declara Ratatui 0.30 sin funcionalidades por defecto; el navegador usa Ratzilla.

## Fase 1: auditar el recorrido actual

### Trabajo

1. Trazar desde el gesto de hablar hasta captura, cierre de turno, transcripción, generación, audio recibido y reproducción local.
2. Revisar por separado conversación normal y llamada en tiempo real. Registrar qué modos están implementados y cuáles pueden validarse; no ampliar su alcance.
3. Para cada etapa, identificar fuente de verdad, evento, actualización de UI y condición de salida.
4. Revisar cancelación, interrupción, fallo de micro, ausencia de voz, fallo de proveedor y desconexión. Comprobar qué controles existen hoy y si quedan accesibles.
5. Distinguir el estado del servidor de la captura y reproducción efectivas en el cliente. Recibir `Speaking` o audio no prueba que ya sea audible.

### Archivos de entrada

- `crates/tipsy-protocol/src/lib.rs`
- `crates/tipsy-assistant/src/session.rs`, `brain.rs`, `client.rs`
- `crates/tipsy-assistant/src/client/`, `audio/`, `realtime/`
- `crates/tipsy-ui/src/state.rs`, `policy.rs`
- `crates/tipsy-web/src/lib.rs`, `hud.rs`, `mic.rs`, `playback.rs`, `web_audio.rs`

Localizar símbolos y llamadas mediante el grafo MCP cuando esté disponible; usar búsquedas dirigidas si no lo está. Revisar el checkout actual antes de editar.

### Entregable y salida

Una tabla con: etapa, fuente real, evento/señal, estado visible, control disponible, evidencia y hueco detectado. Separar código existente de funcionamiento observado. Clasificar los hallazgos por impacto en la conversación.

No avanzar al diseño definitivo si el avatar puede mostrar escucha o habla sin una señal fiable. Si hace falta corregir un fallo de audio, describirlo y acotar una tarea propia antes de mezclarlo con efectos.

## Fase 2: contrato visual del avatar

Definir una tabla de comportamiento a partir de la fase 1. Propuesta inicial, sujeta a las señales verificadas:

| Situación | Intención visual | Condición que debe verificarse |
| --- | --- | --- |
| Reposo | Movimiento suave y presencia estable | Sin captura o reproducción activas |
| Escucha | Reacción al nivel de entrada y marca explícita de escucha | Captura local activa; distinguir silencio de micrófono no disponible |
| Transcripción | Movimiento de espera reconocible | Fin de captura y transcripción pendiente |
| Pensamiento | Actividad visual diferenciada | Generación pendiente, sin fingir porcentaje de progreso |
| Habla | Pulso ligado a la reproducción | Posición/envolvente de audio local, no solo audio recibido |
| Fallo | Señal estable y explicación legible | Evento o condición real, con vía de recuperación |

Resolver prioridades cuando coinciden señales, especialmente escucha durante reproducción, llamada en tiempo real, reconexión y error. No inventar un único orden global antes de estudiar los modos.

Documentar entrada y salida de cada transición, región afectada y comportamiento con movimiento reducido. El color debe acompañarse de texto o forma para comunicar estados. Las etiquetas deben respetar el idioma del perfil.

Salida: contrato visual revisable sin código, incluyendo errores y cancelación. Conservar la identidad del núcleo existente.

## Fase 3: experimento visual mínimo

### Escenarios

Reutilizar el renderizador y el mecanismo de capturas de `xtask/src/shots.rs` donde sirvan. Preparar una secuencia determinista con reposo, escucha con silencio y voz, transcripción, pensamiento, reproducción, cancelación y fallo. Usar datos sintéticos explícitamente etiquetados.

Comparar por separado:

- Núcleo actual con cambios mínimos mediante `Canvas`, colores y geometría existentes.
- El mismo núcleo con una única transición regional de TachyonFX, si su revisión técnica permite continuar.

No crear un framework de animación ni un servidor de demostración nuevo. La reproducción animada debe usar el ciclo de renderizado existente; las capturas estáticas solo sirven para composición, no para demostrar fluidez o sincronización.

### Puerta para TachyonFX

Antes de incorporarlo: elegir una versión publicada concreta, revisar licencia y manifiesto, compatibilidad de `ratatui-core`, funcionalidades WASM y coste de dependencias. La rama de desarrollo consultada es una pista, no la versión aprobada.

Evitar DSL y otras funcionalidades si el efecto no las necesita. Conservar el renderizado puro: el estado temporal del efecto pertenece a la presentación de cada cliente, o a una representación compartida que se justifique; no guardar dispositivos, sockets ni relojes del sistema en `tipsy-ui`.

Aplicar el efecto a la región del avatar. No mover visualmente controles sin actualizar sus áreas de interacción. No animar la desaparición de errores, indicadores de captura o llamada activa.

Salida: comparación visible A/B en terminal y navegador, junto a dependencias y diferencias observadas. Elegir la alternativa más pequeña que comunique mejor los estados; descartar TachyonFX si no aporta una ventaja clara.

## Fase 4: jerarquía y adaptación de pantalla

Mantener el avatar como centro de la experiencia. Estudiar dimensiones reales antes de fijar umbrales:

- Pantalla amplia: avatar protagonista, osciloscopio debajo y complementos laterales discretos.
- Pantalla estrecha: reducir o reubicar métricas y chat antes que el avatar.
- Pantalla muy pequeña: conservar controles esenciales, estado legible y una representación viable del avatar.

No ocultar avisos accionables, estado del micrófono o llamada activa. Mantener acceso a texto como alternativa cuando no se puede hablar. Recalcular áreas de ratón junto con el layout.

Salida: decisiones de layout y capturas en varias dimensiones, incluidos terminal estrecho y navegador. No rediseñar configuración ni login salvo lo imprescindible para preservar su comportamiento.

## Fase 5: implementación elegida y validación

Implementar solo el resultado elegido de las fases anteriores. Revisar todos sus consumidores y actualizar pruebas/capturas pertinentes. No cambiar el protocolo salvo que una carencia demostrada lo exija; si ocurre, tratar compatibilidad como tarea explícita.

### Comprobaciones automatizadas

- Pruebas pertinentes de estado, políticas y renderizado; casos de transición y regresión que puedan fallar de forma significativa.
- Compilación del cliente nativo y del objetivo WASM según el flujo existente.
- Capturas deterministas de estados y tamaños.
- Verificación de que las dependencias visuales no introducen backend nativo en el grafo WASM.

Usar el directorio de build de `.env` mediante el arranque existente cuando corresponda; evitar reconstrucciones en E: sin revisar espacio. Respetar el paralelismo limitado documentado para esta máquina.

### Comprobaciones manuales

Terminal y navegador: iniciar escucha, hablar, cerrar turno, oír respuesta, parar/interrumpir según los controles existentes, repetir, redimensionar y provocar un fallo recuperable. Comprobar permisos del micro y reproducción real.

Comparar antes/después con el mismo escenario y condiciones. Registrar, cuando sea observable, fin de captura, llegada de primer audio y comienzo de reproducción local. No atribuir al renderizador variaciones causadas por red o proveedor. No inferir latencia audible a partir de una captura de pantalla.

### Aceptación

- El avatar conserva protagonismo y representa señales reales durante toda la conversación.
- Las transiciones no bloquean captura, controles ni reproducción.
- Cancelación y final de reproducción no dejan al avatar hablando indefinidamente.
- Un fallo queda visible y permite entender cómo recuperar la conversación.
- Los complementos no desplazan al avatar en pantallas pequeñas.
- Terminal y navegador pasan las comprobaciones pertinentes; los resultados manuales quedan registrados.

No declarar completada la validación de voz real si solo se ejecutaron simulaciones. Si falta autorización o acceso a dispositivos, registrar esa limitación sin presentar el resultado como comprobado.

## Orden de ejecución y condición de parada

Fase 1 → fase 2 → fase 3 → fase 4 → fase 5.

Primer trabajo ejecutable: auditoría de solo lectura de la fase 1. No iniciar por instalar TachyonFX.

Parar cuando una mejora pequeña del avatar y su jerarquía supere los criterios anteriores. Scroll, textarea, Markdown, métricas nuevas y fondos animados quedan fuera de este ciclo.

## Fuentes de evaluación

- https://github.com/ratatui/awesome-ratatui
- https://github.com/ratatui/tachyonfx
- https://github.com/ratatui/tachyonfx/blob/development/Cargo.toml
- https://github.com/ratatui/tui-widgets

Plan elaborado con inspección parcial del código y la spec existente. No se ha ejecutado la auditoría completa, instalado componentes ni validado audio o animación en vivo.
