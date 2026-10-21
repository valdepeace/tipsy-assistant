# Evaluación de componentes Ratatui para Tipsy

Fecha: 2026-10-16
Estado: propuesta para analizar; no aprobada para implementar.
Fuente de descubrimiento: https://github.com/ratatui/awesome-ratatui

## Decisión de producto: conversación por voz y avatar central

Confirmada por el usuario el 2026-10-16. Esta decisión prevalece sobre el orden de prioridades inicial de esta spec y sobre la propuesta anterior de dar más protagonismo al chat.

Tipsy es una experiencia de conversación por voz: hablar y que el asistente nos hable es su función principal. El núcleo del centro es nuestro avatar, no un adorno del dashboard. Toda decisión de diseño, interacción y rendimiento debe favorecer una conversación fluida y mantener al avatar como protagonista.

- El avatar central debe comunicar con claridad reposo, escucha, procesamiento, habla y fallo, conservando su identidad visual.
- El chat, las métricas y el osciloscopio situado debajo del avatar son elementos secundarios de apoyo o decoración. No deben competir con el avatar ni determinar la distribución principal.
- En ventanas estrechas, reducir o reubicar primero los elementos secundarios; no convertir la experiencia en un chat de texto con un avatar residual.
- Los efectos visuales deben acompañar la conversación sin retrasar captura, transcripción, generación, reproducción ni respuesta a los controles.
- El estado mostrado debe corresponder a lo que realmente ocurre: una animación decorativa no prueba que el micrófono esté capturando ni que se esté reproduciendo audio.
- Conservar controles accesibles y estados de error legibles. El carácter secundario del chat no elimina su utilidad como alternativa cuando no se puede usar voz.

“Optimizada al 100 % para conversar” expresa la prioridad de producto, no una garantía de rendimiento ya medida. La evaluación deberá observar el recorrido completo: empezar a hablar, capturar, terminar el turno, recibir respuesta y oírla; además de la sincronización del avatar y los controles de parada/interrupción que estén disponibles.

### Orden de análisis revisado

1. Revisar el recorrido de conversación por voz y cómo el avatar expresa cada estado real, identificando esperas, fricciones y fallos antes de proponer cambios.
2. Evaluar componentes y efectos Ratatui para el avatar y sus transiciones. Conservar el núcleo existente como punto de partida; TachyonFX es un candidato, no una elección aprobada.
3. Ajustar jerarquía y adaptación de pantalla para mantener el avatar central y los controles de voz utilizables.
4. Revisar osciloscopio, chat y métricas como complementos; mejoras de edición, scroll y Markdown quedan subordinadas a la experiencia de voz.

No se aprueba ampliar la infraestructura de audio ni añadir funcionalidades mediante esta decisión. Los posibles cambios se concretarán tras el análisis.

## Objetivo

Optimizar la capa visual para una conversación por voz cuyo protagonista es el avatar central, mediante componentes existentes y conservando la UI compartida entre terminal y navegador. La edición de preguntas y la lectura de respuestas largas son mejoras secundarias. Este documento define qué evaluar; no autoriza añadir dependencias ni modificar el comportamiento actual.

## Situación actual comprobada

- El workspace declara Ratatui 0.30 con `default-features = false`.
- `tipsy-ui` contiene el estado y el renderizado compartidos; `tipsy-web` usa Ratzilla.
- La entrada de preguntas usa un `String` con inserción y borrado al final (`state.rs`, `on_key`).
- El historial usa `List` y ajuste de líneas propio (`render.rs`, `draw_log`, `log_item`, `wrap`).
- El HUD ya dibuja un núcleo animado, ondas de voz y gráficas. No es necesario reemplazarlos.

Referencias locales: `Cargo.toml`, `crates/tipsy-ui/src/state.rs`, `crates/tipsy-ui/src/render.rs`, `crates/tipsy-web/Cargo.toml`.

## Candidatos iniciales (prioridades sustituidas por la decisión de producto)

| Prioridad | Área | Candidato | Resultado buscado |
| --- | --- | --- | --- |
| P1 | Entrada de preguntas | ratatui-textarea | Editar en cualquier posición, seleccionar, deshacer y gestionar varias líneas |
| P1 | Conversación | Widgets nativos de Ratatui; tui-scrollview si aportase una ventaja clara | Leer respuestas mayores que el panel mediante scroll por líneas |
| P2 | Transiciones | tachyonfx | Mejorar apertura de configuración y cambios de estado sin ocultar información |
| P3 | Respuestas enriquecidas | ratatui-markdown u otra alternativa mínima | Mostrar Markdown y bloques de código con legibilidad |

P1 y P2 pueden evaluarse de forma independiente. Markdown debe decidirse después de resolver la navegación del historial.

## Propuesta funcional

### Entrada de preguntas

Evaluar un editor reutilizable en el campo principal de preguntas. No sustituir inicialmente los campos de configuración ni el login.

Comportamiento deseado:

- Insertar y borrar en la posición del cursor; navegar por caracteres y palabras.
- Seleccionar texto y deshacer/rehacer.
- Permitir preguntas de varias líneas y pegado sin enviar accidentalmente.
- Mantener una acción explícita de envío y el control del micrófono.

Decisiones pendientes:

- Resolver la convivencia de las flechas del editor con las que cambian pestañas y seleccionan mensajes.
- Elegir el gesto para salto de línea y envío; comprobar qué combinaciones llegan realmente en terminal y navegador.
- Definir foco de entrada frente a foco del historial, y la altura máxima del editor.
- Comprobar si el modelo de eventos compartido puede representar las operaciones necesarias.

No asumir que copiar los atajos por defecto de la biblioteca conserva el comportamiento de Tipsy.

### Lectura de conversaciones

Evaluar primero una solución con los widgets nativos ya disponibles. Adoptar `tui-scrollview` solo si simplifica el resultado completo.

Comportamiento deseado:

- Desplazarse por una respuesta más alta que el panel, sin perder líneas.
- Poder volver al último mensaje con una acción explícita.
- Conservar la selección de mensaje y su reproducción con F2.
- No desplazar automáticamente al final cuando el usuario esté leyendo mensajes anteriores.
- Ajustar el contenido al redimensionar el panel.

Decisiones pendientes: navegación por teclado y rueda, seguimiento de mensajes nuevos, separación entre mensaje seleccionado y posición de lectura, y representación de bloques de código anchos.

### Transiciones visuales

Evaluar un único efecto breve de TachyonFX, limitado a una región del HUD. Primer candidato: apertura/cierre de configuración o cambio de perfil.

- Aplicar el efecto después de renderizar el contenido.
- Mantener legibles errores, texto de entrada y estados del micrófono.
- No bloquear interacción ni alterar temporización de audio.
- No añadir fondos animados ni reemplazar el núcleo o las ondas existentes.
- Definir cómo se desactivan o reducen los efectos antes de extender su uso.

### Markdown y código

Evaluación posterior, independiente de la voz:

- Renderizar listas, énfasis y bloques de código sin modificar el texto original del mensaje.
- Mantener separados el renderizado visual y la limpieza del texto enviado a síntesis de voz.
- Evaluar coste y utilidad del resaltado antes de activarlo.
- Dejar Mermaid, imágenes y árboles plegables fuera del primer alcance.

El manifiesto consultado de `ratatui-markdown` declara Ratatui `^0.30` y licencia `SySL-1.0`. Es un dato de investigación, no una aprobación de compatibilidad ni de licencia. Revisar términos y alternativas antes de elegirlo.

## Condiciones técnicas para adoptar una biblioteca

1. Seleccionar una versión publicada concreta; revisar su manifiesto y licencia, no solo la rama principal.
2. Comprobar compatibilidad con la versión de Ratatui que resuelve el workspace y evitar tipos incompatibles por versiones duplicadas.
3. Mantener `tipsy-ui` independiente del backend. No introducir Crossterm en el grafo WASM por funcionalidades por defecto.
4. Adaptar eventos nativos y del navegador al modelo compartido, sin duplicar reglas de interacción.
5. Revisar dependencias transitivas, funcionalidades opcionales y coste de compilación/bundle.
6. Preferir el componente individual y desactivar funcionalidades innecesarias.

`ratatui-textarea` documenta una API independiente del backend, pero activa Crossterm por defecto. TachyonFX documenta soporte WASM. Ninguno de esos datos sustituye una compilación y prueba reales en Tipsy.

## Superficie que debe revisarse antes de implementar

- `crates/tipsy-ui/src/state.rs`: entrada, foco, selección y acciones.
- `crates/tipsy-ui/src/render.rs`: campo de preguntas, historial y composición visual.
- `crates/tipsy-protocol`: representación de eventos, solo si el contrato actual resulta insuficiente.
- `crates/tipsy-assistant/src/client`: traducción de eventos nativos y ciclo de renderizado.
- `crates/tipsy-web/src`: traducción de eventos del navegador y ciclo de renderizado.
- Manifiestos afectados y `Cargo.lock`.
- Pruebas existentes de estado/renderizado, capturas generadas y documentación de atajos.

Esta lista es una guía de revisión, no una orden de modificar todos esos archivos.

## Criterios de aceptación de una futura implementación

- Terminal y navegador conservan envío, micrófono, configuración, login y reproducción F2.
- Una pregunta con varias líneas, acentos y caracteres anchos puede editarse sin corrupción ni envío involuntario.
- Una respuesta más alta que el panel puede recorrerse completa y sigue siendo legible tras redimensionar.
- Llegar un mensaje nuevo no interrumpe la lectura del historial anterior.
- Los efectos no ocultan errores ni impiden escribir o controlar el micrófono.
- Pasan las pruebas pertinentes de estado/renderizado y la compilación WASM del proyecto.
- Se comprueban manualmente teclado, pegado, scroll y redimensionado en terminal y navegador.
- El informe registra qué se comprobó, qué queda pendiente y cualquier limitación conocida.

No fijar porcentajes de rendimiento ni presupuestos arbitrarios: medir primero el comportamiento actual y el candidato con el mismo escenario.

## Evaluación mínima y decisión

Para cada área, registrar:

- Problema concreto y ejemplo reproducible en la UI actual.
- Solución con lo ya instalado frente al candidato externo.
- Versión, licencia, funcionalidades y dependencias necesarias.
- Archivos realmente afectados y conflictos de interacción.
- Resultado de compilación nativa/WASM y validación manual.
- Decisión: adoptar, resolver con Ratatui nativo o posponer; con motivo breve.

Orden recomendado: recorrido de voz y estados del avatar → efectos del avatar → jerarquía visual y adaptación de pantalla → elementos secundarios. La edición, el scroll y Markdown no lideran esta evaluación.

## Fuera de alcance

Cambiar proveedores de IA o audio, añadir integraciones Jira/Git, reemplazar Ratzilla, migrar a un framework de UI, incorporar un editor de código completo, vídeo, fondos Matrix, nuevos sistemas de temas o un panel de diagnóstico.

`tui-equalizer`, QR y otros componentes de `tui-widgets` quedan como referencias futuras. Un equalizer necesita datos adecuados: incorporar el widget no calcula por sí mismo un espectro de audio.

## Fuentes revisadas

- Catálogo: https://github.com/ratatui/awesome-ratatui
- Editor y opciones de backend: https://docs.rs/ratatui-textarea/latest/ratatui_textarea/
- Scroll: https://docs.rs/tui-scrollview/latest/tui_scrollview/
- Efectos: https://github.com/ratatui/tachyonfx
- Componentes individuales: https://github.com/ratatui/tui-widgets
- Markdown: https://github.com/celestia-island/ratatui-markdown
- Manifiesto Markdown: https://github.com/celestia-island/ratatui-markdown/blob/master/Cargo.toml

Las fuentes se consultaron el 2026-10-16. Versiones y funcionalidades deben verificarse de nuevo al retomar el análisis.

## Validación de esta spec

Documento basado en inspección del código local y documentación upstream. No se instalaron componentes ni se probó su integración, rendimiento o apariencia en Tipsy.
