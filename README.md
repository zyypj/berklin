# Berklin

Painel e controle remoto do Claude Code para Windows e Android, sem Remote Control.

- Vê todas as sessões do PC (CLI e app desktop) em tempo real: respostas, pensamentos, comandos, diffs de código.
- Responde perguntas (`AskUserQuestion`), aprova ou nega permissões e continua conversas pelo celular.
- Abre chats novos e retoma sessões antigas em segundo plano.
- Agenda tarefas do Claude e lembretes, com notificação no início e no fim.
- O Claude pode chamar você: ferramentas MCP `notify_user`, `ask_user`, `schedule_task` e `list_schedules`.

```
 Claude Code ──hooks──▶ berklin-hook.exe ──HTTP 127.0.0.1──▶ Berklin.exe (agente + app Windows)
 ~/.claude/projects/*.jsonl ──leitura incremental──────────▶ Berklin.exe
 Celular ◀──WebSocket, AES-256-GCM ponta a ponta──▶ relay (Pterodactyl) ◀──▶ Berklin.exe
```

O relay só repassa bytes criptografados; a chave existe apenas no PC e nos celulares pareados. Detalhes do protocolo em [PROTOCOL.md](PROTOCOL.md).

## Como as mensagens chegam no Claude

| Situação da sessão | O que acontece com a sua mensagem |
|---|---|
| Modo ausente ligado e o Claude acabou de terminar | Entra **ao vivo** na mesma sessão aberta no PC (hook `Stop`). |
| O Claude está trabalhando numa sessão aberta no PC | Fica na fila e entra ao vivo assim que ele parar. |
| Sessão em segundo plano aberta pelo Berklin | Vai direto para o processo. |
| Sessão parada ou fechada | Retoma a sessão em segundo plano (`claude -p --resume`). |

**Modo ausente** liga sozinho após N minutos sem mexer no PC (padrão 5) e desliga quando você volta. Com ele ligado, perguntas, permissões e finais de tarefa vão para o celular em vez da tela do PC. Pelo celular dá para ligar e desligar manualmente.

## Download

Tudo fica em [Releases](https://github.com/zyypj/berklin/releases/latest):

| Arquivo | Para quê |
|---|---|
| `berklin-<versão>-windows-x64.zip` | App do Windows (`berklin.exe` + `berklin-hook.exe`) |
| `berklin-<versão>-android.apk` | App do Android |
| `berklin-<versão>-relay-server.js` | Relay em arquivo único, sem dependências |

### Atualização automática

- **Windows:** o app consulta a última release a cada 6 horas, baixa o zip e troca os executáveis sozinho quando nada estiver esperando você. Dá para desligar ou forçar em **Ajustes → Atualizações**.
- **Android:** o app consulta a cada 6 horas, baixa o APK e mostra uma notificação; um toque instala (o Android sempre pede confirmação). Na primeira vez, permita que o Berklin instale apps.

## Instalação

### 1. Relay no Pterodactyl (qualquer egg Node.js 18+)

1. Crie um servidor com uma imagem Node.js e uma porta alocada.
2. Envie o `berklin-<versão>-relay-server.js` como `server.js` e use `node server.js` como comando de início.
3. No primeiro start o relay cria `token.txt` com um token aleatório (ou use a variável `RELAY_TOKEN`). A porta vem de `SERVER_PORT`.
4. Confira em `http://IP:PORTA/health` (responde `ok`). A URL do relay é `ws://IP:PORTA/ws`.

Se o painel tiver proxy com TLS, use `wss://seu-dominio/ws`.

### 2. Windows

Extraia o zip da release em `%LOCALAPPDATA%\Programs\Berklin` e abra o `berklin.exe`. Para compilar e instalar a partir do código:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

O script compila tudo, instala em `%LOCALAPPDATA%\Programs\Berklin`, cria o atalho no Menu Iniciar e abre o app. Na primeira execução o app:

- adiciona os hooks em `~/.claude/settings.json` (com backup em `settings.json.berklin-backup`);
- registra o MCP `berklin` no Claude Code (`claude mcp add --scope user`);
- passa a iniciar com o Windows, minimizado na bandeja.

Com o app fechado, os hooks não fazem nada (saem em milissegundos). Para remover: **Ajustes → Integração → Remover**.

Depois, em **Ajustes**, preencha a URL e o token do relay e salve. O QR code de pareamento aparece em seguida.

Para chats em segundo plano e agendamentos, o `claude` de linha de comando precisa estar logado uma vez:

```powershell
claude auth login
```

### 3. Android

1. Instale o `berklin-<versão>-android.apk` da release.
2. No app, toque em **Escanear QR code do PC** e aponte para o QR code em **Ajustes** no Berklin do PC. A câmera do sistema ou o Google Lens também funcionam (o link `berklin://pair…` abre o app já pareado), e dá para colar o link.
3. Permita as notificações e, em **Ajustes → Abrir ajuste de bateria**, tire o Berklin da otimização de bateria.

As notificações têm ações diretas: **Permitir / Negar**, as opções da pergunta e **Responder** com texto.

## Desenvolvimento

| Parte | Pasta | Build |
|---|---|---|
| Agente, hook e app Windows (Rust, Tauri 2) | `crates/core`, `crates/hook`, `app` | `cargo build --release` |
| Interface (Preact, compartilhada) | `ui` | `npm run build` (gera `ui/dist`) |
| Android (Kotlin, OkHttp, sem AndroidX) | `android` | `gradlew.bat assembleDebug` (copia `ui/dist` para o APK) |
| Relay (Node, `ws`) | `relay` | `node relay.js`, ou `node build.mjs` para gerar `dist/server.js` |

Para gerar os arquivos de uma release: `scripts\release.ps1` (saída em `dist-release/`). O APK de release é assinado com a chave apontada por `android/keystore.properties`, que fica fora do repositório; sem ela, as atualizações do Android não instalam por cima.

Variáveis úteis para testes: `BERKLIN_DATA_DIR` (pasta de dados isolada), `BERKLIN_NO_AUTOINSTALL=1` (não mexe nos hooks), `BERKLIN_IGNORE_IDLE=1` (desliga o modo ausente automático).

Os dados ficam em `%APPDATA%\Berklin` (`config.json`, `schedules.json`, `index.json`, `berklin.log`).

## Limites conhecidos

- Uma sessão aberta no PC e parada, com o modo ausente desligado, não pode receber mensagem ao vivo. Nesse caso ela é retomada em segundo plano, e a conversa na janela do PC só mostra isso depois de reaberta.
- O Claude Code limita a 8 continuações seguidas pelo hook `Stop`. O Berklin para na 7ª e avisa; a mensagem seguinte retoma em segundo plano.
- O formato dos transcripts (`.jsonl`) é interno do Claude Code e pode mudar entre versões.
- O app Android ainda não foi testado num aparelho real.
- Quem instalou o APK de debug precisa desinstalá-lo uma vez antes do APK da release, porque as assinaturas são diferentes.
