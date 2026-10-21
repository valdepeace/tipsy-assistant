# Ventana flotante del núcleo — borrador de especificación

Estado: borrador; el comportamiento compacto se está validando en un spike aislado.  
Fecha: 2026-10-21.

Plan de trabajo: [plan de ventana flotante](2026-10-21-floating-core-window-plan.md).

## Objetivo

Añadir una ventana compacta de Tipsy que pueda permanecer en una esquina, encima de las ventanas normales, y seguir aceptando interacción cuando la ventana principal pierde el foco.

## Invariante de compatibilidad

La ventana flotante amplía el producto. No elimina, reemplaza ni cambia el comportamiento de las superficies actuales:

- El cliente TUI nativo sigue arrancando y funcionando como hasta ahora.
- `tipsy-assistant serve`, el cliente web y el terminal web conservan sus rutas y contratos actuales.
- La nueva superficie se inicia de forma explícita y puede cerrarse sin cerrar ni alterar los clientes existentes.
- No se cambia el protocolo ni el comportamiento de audio de los clientes existentes para habilitar la ventana.

## Alcance inicial propuesto

- Solo Windows para el primer prototipo.
- Una ventana compacta muestra el núcleo/avatar y el osciloscopio de voz debajo; un clic en esa vista activa o detiene el micrófono mediante el atajo existente F3.
- Al perder el foco, la ventana pasa a modo compacto y se coloca en la esquina elegida; permanece por encima de ventanas normales.
- Al recuperar el foco, vuelve a la vista completa. El usuario puede volver a compactarla sin cerrar la sesión.
- La ventana sigue siendo interactiva. No se convierte en una superposición que atraviese clics.
- El usuario puede moverla y volver a la vista completa mediante una acción visible.
- Cerrar la ventana termina únicamente esa superficie y sus recursos propios.
- La primera versión crea una sesión independiente del cliente TUI que ya pudiera estar abierto. No intenta compartir ni sincronizar una conversación entre procesos.

## Restricciones de integración conocidas

- `tipsy-ui` comparte el renderer Ratatui entre el cliente nativo y el cliente WebAssembly; `tipsy-web` usa navegador para WebSocket, micrófono y reproducción.
- El cliente web deriva `/ws` del origen de la página y el servidor valida `Origin`. Una ventana de escritorio que reutilice ese cliente deberá conservar un origen HTTP local compatible; abrir el HTML como archivo o cambiarlo a un origen `tauri://` no está demostrado como compatible.
- `portable-pty` ya cubre pseudoterminales para el terminal web. No es un gestor de ventanas ni un renderer del HUD, por lo que no se presupone que la ventana flotante deba ser una terminal.
- Tauri/WebView2 es una hipótesis de contenedor, no una decisión aprobada. Permisos de micrófono, audio, WebGL2 y empaquetado deben validarse en un prototipo.

## Fuera de alcance

- Reemplazar el cliente TUI o cambiar su arranque por defecto.
- Crear un emulador de terminal nuevo.
- Añadir sincronización de estado entre varias instancias o clientes.
- Rediseñar el HUD completo o definir ahora el arte final del avatar compacto.
- Garantizar visibilidad sobre UAC, escritorios seguros, ventanas a pantalla completa exclusivas u otras superficies que Windows no permita cubrir.

## Criterios de aceptación

1. El arranque actual de TUI y sus controles siguen funcionando sin cambios observables.
2. La ventana nueva se inicia mediante una entrada explícita (provisionalmente, `tipsy-assistant window`); el comando sin subcomando mantiene su comportamiento actual.
3. En Windows, la ventana se mantiene encima de ventanas normales tras perder el foco; en compacto se ve el núcleo y su osciloscopio, y un clic en la vista controla el micrófono.
4. La transición a compacto y el retorno a vista completa conservan la sesión de esa ventana; no aparece una segunda sesión por el cambio de tamaño o foco.
5. Micrófono, reproducción de voz y reconexión funcionan en la ventana de escritorio; cualquier permiso denegado se comunica y permite recuperarse.
6. Cerrar la ventana libera sus recursos y deja el modo actual disponible para una nueva ejecución.
7. La ventana puede coexistir con una ejecución TUI independiente; cada ejecución conserva su sesión y sus propios recursos de audio.

## Decisiones pendientes

- Confirmar el comando nuevo `tipsy-assistant window` como punto de entrada.
- Confirmar una sola ventana por ejecución, que cambia entre tamaño completo y compacto; no crear una segunda ventana ni una segunda sesión en el cambio de foco.
- El control compacto elegido para el primer corte es un clic en la vista, conectado al toggle existente F3; al recuperar foco vuelve la vista completa y sus controles.
- Definir esquina inicial, movimiento manual y persistencia de posición. Propuesta inicial: esquina inferior derecha del monitor donde se abre, posición ajustable con arrastre; no persistir todavía.
- ¿Se reutiliza la web WASM en WebView2 o se construye un frontend nativo sobre `tipsy-ui`? Elegir tras el prototipo de micrófono, WebSocket y ajuste de tamaño.
- Definir si el modo `window` inicia un servidor loopback propio o exige uno ya activo. Propuesta inicial: iniciar y apagar el servidor loopback propio junto con la ventana, sin apropiarse de un servidor externo.

## Plan de validación

1. Probar una ventana WebView2 mínima con contenido local servido por Tipsy, topmost, cambio de tamaño, foco y posición.
2. Validar micrófono, audio y WebSocket contra el servidor local existente.
3. Comparar la reutilización de la UI web con una ventana nativa solo si el primer camino falla en un requisito concreto.
4. Mantener una comprobación de regresión de los comandos TUI, web y servidor existentes.

La aceptación visual y de hardware requiere observación manual en Windows; una compilación no prueba el micrófono ni el sonido real.
