# melodias-studio

TUI en Rust que convierte una canción de YouTube en dos entregables para el canal: **una miniatura** y **un track instrumental** de 3 minutos, usando solo modelos que corren en la P40 de la casa.

```
URL de YouTube + vibe
        │
        ├─ yt-dlp ─────────► audio fuente
        ├─ ffmpeg ─────────► extracto de 60 s con más energía (referencia de timbre)
        ├─ llama-server ───► prompt de miniatura
        ├─ audiocpp_server ► toma de 210 s con reference_audio
        ├─ ffmpeg ─────────► loop de 3 s + loudnorm -15 LUFS
        └─ sd-server ──────► miniatura 1280x720
```

## Requisitos

- Rust 1.85+ (`rustup default stable`).
- `yt-dlp`, `ffmpeg`, `ffprobe`, `scp` y `ssh` en el `PATH`.
- Los tres servicios levantados en la VM de la GPU:
  - `audiocpp_server` (ACE-Step) en `:8090`
  - `sd-server` (MOSS-Image) en `:8081`
  - `llama-server` (Qwen) en `:8080`
- Acceso SSH sin password al host de la VM, para dejar la referencia de audio del lado del server.

La P40 tiene 24 GB, así que los modelos no conviven: la app le pide a `audiocpp_server` que libere VRAM antes de generar la miniatura.

## Uso

```bash
cargo run
```

- `Tab` alterna entre la URL y el vibe.
- `Enter` avanza de campo y después arranca la generación.
- `q` sale (solo cuando terminó).

Cada etapa aparece con su estado y el log muestra cada paso, incluido el prompt que escribió el LLM.

## Configuración

Todo por variables de entorno, con defaults apuntando a la VM:

| Variable | Default | Qué hace |
| --- | --- | --- |
| `AUDIO_URL` | `http://192.168.88.38:8090` | `audiocpp_server` |
| `AUDIO_MODEL` | `ace-step` | id del modelo en el server |
| `SD_URL` | `http://192.168.88.38:8081` | `sd-server` |
| `LLM_URL` | `http://192.168.88.38:8080` | `llama-server` |
| `LLM_MODEL` | `local` | nombre que espera el server |
| `SSH_HOST` | `root@192.168.88.38` | destino para copiar la referencia |
| `REMOTE_REF_DIR` | `/models/ref` | carpeta remota de referencias |
| `OUT_DIR` | `output` | salida local |
| `DURATION_SEC` | `210` | largo de la toma |
| `STEPS` | `32` | pasos de difusión |
| `GUIDANCE` | `5` | CFG |
| `SAMPLER` | `heun` | `euler` o `heun` |
| `WIDTH` / `HEIGHT` | `1280` / `720` | tamaño de la miniatura |

## Salida

```
output/<slug>/
├── audio.mp3      # loop de 3 min, 320 kbps, -15 LUFS
├── thumbnail.png  # 1280x720
├── meta.json      # seed, prompt, parámetros y URL de origen
└── work/          # fuente, referencia y toma intermedia
```

## Decisiones

- **La referencia es un extracto, no la canción entera.** ACE-Step toma tres ventanas de 10 s del archivo, así que se elige el minuto con mayor energía promedio y se evitan intros y breakdowns.
- **El loop se arma en ffmpeg.** Crossfade de 3 s entre el final del outro y el inicio del intro. No depende del modelo ni de su tempo.
- **La miniatura la escribe el LLM.** El vibe entra al chat como concepto y vuelve como prompt en inglés, con el estilo pixel art fijado en el system prompt.
- **La VRAM se libera explícitamente.** `unload_all_models` en audio.cpp antes de pedir la imagen, porque los dos modelos no entran juntos.

## Roadmap

- [ ] Detección de tempo y corte alineado al compás (hoy el crossfade es fijo).
- [ ] Cancelar una generación en curso y reintentar una etapa sola.
- [ ] Leer el título del video para nombrar la carpeta, en vez del vibe.
- [ ] Config por archivo (`~/.config/melodias-studio/config.toml`).
- [ ] Escribir directo la ficha en `jd-llm/projects/melodias8bit/videos/`.
- [ ] Comparar dos tomas lado a lado antes de quedarse con una.
