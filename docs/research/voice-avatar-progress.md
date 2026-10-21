# Progreso: voz y avatar central

Backlog: [tareas para Luna](2026-10-16-voice-avatar-tasks.md).

| ID | Estado | Entregable/evidencia | Comprobaciones | Pendientes |
| --- | --- | --- | --- | --- |
| VA-01 | hecha | [voice-avatar-audit.md](voice-avatar-audit.md): rutas nativa/web y convencional/realtime, señales, controles y huecos | Inspección estática; `rg` dirigido por falta de `codebase-memory-mcp`; sin tests/proveedores/dispositivos ejecutados | Falta inspección manual y señal local de progreso/actividad de salida conectada al avatar; continuar solo con VA-02 tras revisión |
| VA-02 | hecha | [voice-avatar-contract.md](voice-avatar-contract.md): prioridades, entradas, combinaciones, textos ES/EN, geometría, transición única y límites | Contrato documental contrastado con VA-01 y símbolos referenciados; sin código, tests, builds o proveedores | Revisión antes de VA-04; salida local agrupada como pendiente/programada/en curso; error de micrófono/reconexión y barge-in no tienen señal común tipada; sin inspección visual manual |
| VA-03 | hecha | [voice-avatar-effects-decision.md](voice-avatar-effects-decision.md): opción nativa frente a TachyonFX 0.25.2; decisión de no añadir dependencia y puntos de estado por cliente | Fuentes primarias actuales consultadas el 2026-10-16; comparación Ratatui/core/WASM/features y evidencia local; documentación únicamente, sin resolver dependencias ni compilar | Decisión nativa recomendada; sin A/B visual ni prueba WASM de TachyonFX en este workspace |
| VA-04 | parcial | Cambios implementados en `tipsy-ui` y alimentadores nativo/web; separación de `local_output` y `status`, capas independientes de Live/mic/salida/aviso, transición nativa 180 ms con liberación al finalizar y niveles históricos fuera del avatar. Header estrecho recorta título y omite output antes de Live/mic para preservar controles. Contrato aclara semántica del error más reciente. | `rustfmt --edition 2024` y `git diff --check` pasan. `cargo check -p tipsy-ui -p tipsy-assistant -p tipsy-web -j 1` pasó: 168 crates, 8m15s, target G:. Prueba `TestBackend` añadida para modo/engranaje visibles y hits dentro de bounds. La suite de tests conjunta fue interrumpida tras ~30 min sin salida; la aislada de `tipsy-ui` también tras ~3 min sin salida durante compilaciones concurrentes; no hubo resultado de tests. No se inició app/proveedor/dispositivo. Revisión de escrituras: producción pasa por `set_mic_open`/`set_live`; escrituras directas restantes son fixtures/tests. | Tests, WASM e inspección visual pendientes. |
| VA-05 | pendiente | — | — | Comparación sintética |
| VA-06 | pendiente | — | — | Inspección visual |
| VA-07 | pendiente | — | — | Layout |
| VA-08 | pendiente | — | — | Validación final |

Usar `en curso`, `hecha`, `parcial` o `bloqueada` al actualizar. Un caso no ejecutado no equivale a una prueba pasada.
