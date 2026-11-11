# LegatIA Design (fork do FigCraft)

Este fork é o [FigCraft](https://github.com/storytold/figcraft) com a cara da LegatIA, usado na página
Design do LegatIAdm. O branch **`legatia`** é a release do FigCraft indicada em `LEGATIA_BASE` mais poucos
commits nossos. O `main` deste fork fica igual ao original.

## O que muda

| Onde | O quê |
|---|---|
| `crates/ui-egui/src/legatia.rs` | As cores da LegatIA (preto, bordas finas, azul #006FFE) sobre os tokens do FigCraft, e o nome "LegatIA" no lugar do logotipo. Arquivo só nosso. |
| `crates/ui-egui/src/theme.rs` | Só ganchos: `apply_with` e `on_dark` passam os tokens por `legatia::tokens`; `BRAND`, `ACCENT` e `ACCENT_ON` são o azul; `paint_wordmark` chama `legatia::paint_wordmark`; `install_fonts` chama `legatia::fonts::install`; o ícone é `legatia-64.png`. |
| `crates/ui-egui/src/lib.rs` | `pub mod legatia;` |
| `crates/ui-egui/src/legatia/host.rs` | A ponte com a página em volta do editor (o LegatIAdm): o botão no lugar de "Compartilhar" (Salvar na LegatIA), botões à esquerda do tema (Sair da tela cheia), os arquivos da LegatIA na tela inicial e o "+" de arquivo novo. Arquivo só nosso. |
| `crates/ui-egui/src/legatia/fonts.rs` | A fonte da interface que a página entrega em tempo de execução (a Satoshi). Arquivo só nosso. |
| `crates/ui-egui/src/legatia/glass.rs` | Os painéis, a barra de abas e a dock como vidro fosco sobre o design: o canvas é desenhado também embaixo deles e desfocado ali, com um vidro translúcido por cima. Arquivo só nosso. |
| `crates/ui-egui/src/app.rs` | Só ganchos: os painéis usam `legatia::panel_frame` (cartões translúcidos, sem a linha do egui ao lado) e `legatia::backdrop` guarda o lugar do design atrás deles. |
| `crates/ui-egui/src/toolbar.rs` | Só gancho: a dock de ferramentas é um cartão de vidro (`legatia::glass::dock`); no tema claro, a de sempre. |
| `crates/ui-egui/src/canvas.rs`, `canvas/render.rs` | Só ganchos para `legatia::glass`: o render cobre também a área dos painéis (`widen`), e cada render vai desfocado para trás deles (`frost`, `page`, `canvas`). |
| `crates/ui-egui/src/widgets.rs`, `panels/layers.rs`, `panels/right.rs` | Só ganchos: as linhas entre as seções dos painéis usam `legatia::divider` (nenhuma); o item ativo da barra da esquerda é uma pílula de vidro claro (`legatia::active_item`). Em `widgets.rs` e `panels/export_panel.rs`, os botões secundários usam `legatia::button_edge` (vidro claro, sem contorno, no tema escuro). |
| `crates/ui-egui/src/panels/right.rs`, `tabs.rs`, `home.rs` | Só ganchos para `legatia::host` e, em `tabs.rs`, para `legatia::glass` (o lugar do vidro antes da barra de abas, e a barra clara sobre ele) (botão principal, botões da barra de abas e "+", arquivos na tela inicial). Sem página que peça, tudo fica como no FigCraft. |
| `apps/figcraft-web/src/api.rs`, `web.rs` | `figcraft.host(...)`, `figcraft.onhost`, `figcraft.thumbnail(bytes)` e `figcraft.uiFonts(regular, bold)` e `figcraft.dockSlot()` (onde fica o lugar da página na dock, `host.dockSlot`) no `window.figcraft` (com `?control`); `toolbar.rs` só soma esse lugar à dock (`legatia::host::dock_slot`); os pedidos do `window.figcraft` também andam com a aba escondida (`logic` em `web.rs`), para o "Conectar Claude" funcionar com o editor em outra aba. |
| `crates/engine/src/automation/inspect.rs` (e o teste `inspect_fields_tests.rs`) | Uma linha: `document.inspect` devolve também `revision`, o contador de mudanças do documento, que o salvamento automático da página usa (os rótulos do desfazer param em 20 passos). |
| `assets/app-icon/legatia-64.png` | O ícone da LegatIA (listado no `ATTRIBUTION.md`). |
| `assets/app-icon/*` (mesmos nomes de arquivo) | O ícone do app é a marca da LegatIA ("IA" em quadrado preto, o favicon do site) no lugar do camaleão: `.ico` do `.exe`, PNGs, `hicolor/` e os SVGs mestres. Licença própria em `assets/app-icon/LICENSE.txt`. |
| `apps/figcraft/build.rs`, `src/main.rs`, `src/startup.rs`, `crates/ui-egui/src/windows.rs` | O nome que a pessoa vê: "LegatIA Design" no título da janela (`<arquivo> – LegatIA Design`), nos diálogos de erro de inicialização e nas propriedades do `.exe`. O nome interno `FigCraft` (pasta de configurações, `APP_ID`, id do eframe) NÃO muda, para não perder as preferências de quem já usa. |
| `crates/ui-egui/src/legatia/sync.rs`, `legatia.rs`, `tabs.rs` | Só desktop: o botão na barra de abas com o estado do arquivo aberto em relação à LegatIA (Na LegatIA · versão N, Enviando…, Em edição por fulano, Conflito, Salvar na LegatIA) e o clique que salva no disco e manda enviar na hora. Fala com o `legatia-sync` (tools/legatia-sync, no repositório do site) em `127.0.0.1:47821` com o segredo de `%APPDATA%\LegatIA Design\native.token`; sem o programa rodando não aparece. Cliente HTTP só com a biblioteca padrão, nenhuma dependência nova; no web o módulo não faz nada. |
| `apps/figcraft-web/index.html` | Título "LegatIA Design", fundo preto desde o primeiro quadro, "Carregando o editor…". |
| `docs/brand/` | Removido: a licença da marca ArtCraft pede que forks tirem os logos. |
| `.github/workflows/legatia-web.yml`, `LEGATIA_BASE`, este arquivo | A atualização automática e o build. |

A fonte embutida continua a Inter (licença aberta). A Satoshi não vai no build porque a licença dela
(ITF Free Font License) não permite redistribuir os arquivos, e a release é pública; o LegatIAdm carrega a
Satoshi do Fontshare em tempo de execução, para os designs (`text.loadFont`) e para a interface (`figcraft.uiFonts`).

## Atualização automática

`.github/workflows/legatia-web.yml` roda todo dia às 06:17 (e quando chamado em Actions ▸ LegatIA web):

1. Se o FigCraft tem release mais nova que `LEGATIA_BASE`, faz o merge dela no `legatia` (`git merge <nova>`)
   e atualiza `LEGATIA_BASE`. Nada de rebase nem force-push: o histórico não é reescrito.
2. Gera a versão web como a release do próprio FigCraft (trunk, craft-fonts na versão que o `release.yml`
   deles fixa, craft-i18n) e publica a release `<base>-legatia.<n>` com o zip e o SHA-256. As notas da
   release trazem o trecho pronto para o `figcraft.web.json` do LegatIAdm.
3. Se uma mudança deles bater nas mesmas linhas que as nossas, nada é publicado: abre uma issue
   "LegatIA: conflito ao atualizar…" com o passo a passo, e a LegatIA continua na versão anterior.

Um push no branch `legatia` (uma mudança nossa) também gera uma release nova na mesma base.

## App para Windows (instalador do time)

Além da versão web, a LegatIA publica o app nativo como `figcraft-<versão>-windows-x64-portable.zip` numa
release **estável** (`v<X.Y.Z>`, sem sufixo, não marcada como pré-lançamento) deste repositório. As releases
da versão web (`v<X.Y.Z>-legatia.<n>`) têm sufixo e o verificador de atualização do app as ignora.

- O zip leva `figcraft.exe`, `figcraft-cli.exe`, `legatia-sync\` (o `node.exe` e o `sync.mjs` do LegatIAdm,
  `npm run design:sync:bundle`) e o `install.ps1` / `Instalar.cmd`. O app abre o sincronizador junto e o encerra ao
  sair (`apps/figcraft/src/legatia_sync.rs`), por isso uma atualização consegue trocar os arquivos.
- `UPDATE_APP` (`crates/ui-egui/src/lib.rs`) aponta para `Lulindos/figcraft`. O aviso de versão nova aparece ao
  abrir o app; "Baixar" fecha, troca os arquivos por cima da pasta e reabre.
- `vendor/craft-update` é a cópia do crate de atualização do craft-libs (rev `1b34eba`) com **uma** mudança: o
  upstream só instala atualização de binário com assinatura digital do mesmo editor, e a nossa build não tem
  certificado. Aqui a garantia é o SHA-256 que o GitHub publica para o asset da nossa release (conferido no
  download). Ao trocar para um app assinado, volte ao crate do craft-libs.
- **Pasta de instalação e MSI:** o `install.ps1` instala em `%LOCALAPPDATA%\Programs\LegatIA Design`, que é a pasta padrão do MSI do
  launcher do craft-libs; sem tratamento, o atualizador achava que a cópia era do MSI e pedia um `.msi` (0.15.x e 0.17.0 têm esse erro,
  só se resolve reinstalando com o zip da 0.17.1 ou mais nova). `vendor/craft-update/src/msi_copy.rs::is_legacy` devolve `false`
  para o nosso repositório (há teste). **Teste de atualização só vale a partir da pasta instalada de verdade**, não de pasta temporária.
- **Janela de atualização:** `legatia::update_text` traduz os textos (o crate usa o inglês como chave) e `vendor/craft-update/src/dialog.rs`
  perde a fonte monoespaçada e os cantos retos; os botões não vão mais em maiúsculas (`translate.rs::button`).
- Para lançar uma versão: suba `[workspace.package] version` (X.Y.Z, maior que a anterior), compile com
  `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'`
  (`cargo build --release --locked -p figcraft -p figcraft-cli --target x86_64-pc-windows-msvc`), rode
  `packaging/windows/team/package-team.ps1 -SyncDir <dist/legatia-sync>` e publique o zip numa release `v<versão>`.
- **Fonte:** a Satoshi não vai no zip (licença). O `install.ps1` roda `legatia-sync\fonts.mjs` (baixa do Fontshare, renomeia a família como o site faz) e instala por usuário em `%LOCALAPPDATA%\Microsoft\Windows\Fonts` como `LegatIA-Satoshi-*.ttf`; o app lê esses arquivos para a interface (`apps/figcraft/src/legatia_fonts.rs`) e os textos dos designs a acham como fonte do sistema.
- **IA (MCP):** os atalhos abrem o app com `--control 47800` (canal só em 127.0.0.1). `connect-ai.ps1` / `Conectar-IA.cmd` registra `figcraft-cli mcp --attach 47800` (`legatia-design`, ao vivo) e `figcraft-cli mcp` (`legatia-design-offline`) no Claude Desktop, Claude Code e Cursor, e mostra o trecho para qualquer outra IA. Não há token nesse canal: qualquer programa do computador pode usá-lo.
- **Botão e orbe (0.15.2):** `legatia/sync.rs::host_frame` entrega ao `host` o botão primário (estado do arquivo no lugar do
  Compartilhar; o botão cinza da barra de abas saiu) e o espaço do orbe, que `legatia/agent.rs` desenha no fim da barra de
  ferramentas. O painel lista as IAs achadas no computador: Claude Code (`claude -p`) e Codex (`codex exec -`) rodam sem janela
  com a ponte MCP indicada na própria chamada e o pedido na entrada padrão; as demais abrem com o pedido copiado.
- **Home (0.17.2):** `legatia/home.rs` desenha a Home do app do computador (saudação em Satoshi Light em duas vozes, perfil em
  pílula de vidro, ações, projetos com os arquivos como cartões de vidro, aurora azul), no lugar da do FigCraft (que segue valendo
  para o navegador). Os dados vêm de `GET /native/overview` do legatia-sync (perfil, projetos e arquivos com estado). Fora da Home:
  a aba "Online", o código de convite e os "Primeiros passos" não aparecem. Os testes do FigCraft rodam sobre a Home dele
  (`cfg!(test)`), e a nossa tem os próprios testes em `legatia::home`. A fonte Light vem de `LegatIA-Satoshi-Light.ttf`
  (`fonts::set_with_light`); sem ela os títulos usam a Regular.
- **Orbe liga a IA sozinho (0.17.2):** na primeira vez que uma IA de janela recebe um pedido, `agent::connect` acrescenta as duas
  entradas da LegatIA à configuração de MCP dela (`add_entries`: texto intacto, cópia `.bak-legatia`, só grava JSON válido).
- **Salvar um arquivo novo (0.17.3):** `legatia/save_dialog.rs` é o quadro "Salvar na LegatIA" (projeto + nome) que `FigApp::save`
  abre para um documento sem caminho, e o botão primário abre para um arquivo "outside". Grava em `LegatIA Design\<pasta do projeto>\<nome>.figcraft`
  com `file.saveAs` e manda `sync-now`; quem cria o arquivo na nuvem é o legatia-sync (createNew). A pasta de cada projeto vem do
  `/native/overview` (`folder`, a mesma `sanitizeName`); `sanitize_name` em Rust espelha a do programa. Os "Outros projetos" da Home são
  clicáveis (`prefer_project`).
- **Sem conexão (0.17.3):** o legatia-sync testa a conexão a cada ciclo (`reachable`, um HEAD de 3 s no `url` do config) e diz `online` em
  `/native/status` e `/native/overview`; sem conexão ele não mexe na rede, o que se salva fica `pending` na pasta e sobe quando ela volta.
  Guarda a última lista em `overview-cache.json` (na pasta de estado) para a Home abrir sem internet. No app, `sync::primary` troca os textos
  ("Salvar neste PC", "Aguardando rede"), o quadro de salvar e a Home avisam, e `sync::offline()` evita mandar enviar.
- **Luz do topo da Home (0.17.3):** `legatia/aurora.rs` porta a Aurora do dashboard (shader simplex, três cores por horário), o campo ASCII por
  cima (rampa, ondas, mistura overlay somada com alfa 0) e o degradê para o fundo, como `Aurora.tsx`, `AsciiField.tsx` e `app/dashboard/page.tsx`
  do site. 15 quadros por segundo; um quadro só com "reduzir movimento".
- **Home em nuvem e desktop (0.17.4):** "Na nuvem" (nuvenzinha azul) lista os projetos do Supabase, cada um numa faixa com a capa
  (`projects.cover_image`, via `/native/cover?project=<id>`); o arquivo ainda sem desenho baixado usa a capa no lugar da miniatura. "Neste computador" são
  os recentes que não estão na nuvem (`home_recents::listed` esconde os da nuvem por `legatia::hidden_recent`). Os ~50 projetos sem arquivo viraram um campo de busca
  (`project_search`, até 8 resultados). As imagens vêm do programa de sincronização por `legatia/images.rs` (fila de 3 trabalhadores, textura na
  primeira vez que se desenha, nova tentativa em 1 min).
- **Fotos (0.17.4):** a foto de quem está conectado troca a bolinha "Y" da barra de pessoas do editor (`legatia::paint_my_avatar`, gancho em `live::people_bar`);
  o arquivo travado mostra a foto de quem o edita (`/native/avatar?user=<id>`, o programa aceita só UUID e só imagem do Supabase da conta).
- **Logo oficial (0.17.4):** `assets/legatia/logo.png` (o `public/logo.png` do site, recortado) no lugar do ícone e do "Legat" + "IA" da Home (`legatia::paint_logo`).
- **Abre na Home (0.17.4):** `main.rs` abre na Home quando nada foi aberto ou recuperado; o arquivo em branco fica atrás para o MCP e a Home
  sai sozinha na primeira vez que um documento ganha arquivo ou alteração (`legatia::leave_home_when_touched`).
- **Orbe envia de verdade ao Antigravity (0.17.4):** `agy -p <pedido> --settings {"permissions":{"allow":["mcp(legatia-design/*)"]}}` (só desta chamada),
  depois de `agy mcp add` das duas entradas com a porta de agora. O pedido manda não rodar comandos de terminal. Claude Desktop, Windsurf, Cursor e VS Code
  não têm como receber texto de fora: seguem copiando e abrindo (o painel diz isso).
- **Ícone:** "IA" com a primeira barra em azul (`#006FFE`), para não se confundir com o app da ilha (`assets/app-icon/README.md`).
- O workflow de Actions deste fork está parado pelo GitHub, então o build é local.

## Regras para mexer aqui

- Personalização nova vai em `legatia.rs` sempre que der; no código do FigCraft, só a linha que chama ela.
  Quanto menos linhas do original tocadas, menos conflito nas atualizações.
- Um token novo do FigCraft que a LegatIA ainda não mapeou fica com o valor deles (`..t` em `legatia.rs`),
  em vez de quebrar o build: vale olhar `Tokens` em `theme.rs` a cada atualização.
- Não suba arquivos de fonte com licença que proíba redistribuir: o repositório e as releases são públicos.
