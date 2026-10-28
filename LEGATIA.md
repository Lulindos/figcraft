# LegatIA Design (fork do FigCraft)

Este fork é o [FigCraft](https://github.com/storytold/figcraft) com a cara da LegatIA, usado na página
Design do LegatIAdm. O branch **`legatia`** é a release do FigCraft indicada em `LEGATIA_BASE` mais poucos
commits nossos. O `main` deste fork fica igual ao original.

## O que muda

| Onde | O quê |
|---|---|
| `crates/ui-egui/src/legatia.rs` | As cores da LegatIA (preto, bordas finas, azul #006FFE) sobre os tokens do FigCraft, e o nome "LegatIA" no lugar do logotipo. Arquivo só nosso. |
| `crates/ui-egui/src/theme.rs` | Só ganchos: `apply_with` e `on_dark` passam os tokens por `legatia::tokens`; `BRAND`, `ACCENT` e `ACCENT_ON` são o azul; `paint_wordmark` chama `legatia::paint_wordmark`; o ícone é `legatia-64.png`. |
| `crates/ui-egui/src/lib.rs` | `pub mod legatia;` |
| `assets/app-icon/legatia-64.png` | O ícone da LegatIA (listado no `ATTRIBUTION.md`). |
| `apps/figcraft-web/index.html` | Título "LegatIA Design", fundo preto desde o primeiro quadro, "Carregando o editor…". |
| `docs/brand/` | Removido: a licença da marca ArtCraft pede que forks tirem os logos. |
| `.github/workflows/legatia-web.yml`, `LEGATIA_BASE`, este arquivo | A atualização automática e o build. |

A fonte da interface continua a Inter (licença aberta). A Satoshi não vai no build porque a licença dela
(ITF Free Font License) não permite redistribuir os arquivos, e a release é pública; o LegatIAdm carrega a
Satoshi do Fontshare no editor para os designs.

## Atualização automática

`.github/workflows/legatia-web.yml` roda todo dia às 06:17 (e quando chamado em Actions ▸ LegatIA web):

1. Se o FigCraft tem release mais nova que `LEGATIA_BASE`, reaplica os commits da LegatIA em cima dela
   (`git rebase --onto <nova> <base> legatia`) e atualiza `LEGATIA_BASE`.
2. Gera a versão web como a release do próprio FigCraft (trunk, craft-fonts na versão que o `release.yml`
   deles fixa, craft-i18n) e publica a release `<base>-legatia.<n>` com o zip e o SHA-256. As notas da
   release trazem o trecho pronto para o `figcraft.web.json` do LegatIAdm.
3. Se uma mudança deles bater nas mesmas linhas que as nossas, nada é publicado: abre uma issue
   "LegatIA: conflito ao atualizar…" com o passo a passo, e a LegatIA continua na versão anterior.

Um push no branch `legatia` (uma mudança nossa) também gera uma release nova na mesma base.

## Regras para mexer aqui

- Personalização nova vai em `legatia.rs` sempre que der; no código do FigCraft, só a linha que chama ela.
  Quanto menos linhas do original tocadas, menos conflito nas atualizações.
- Um token novo do FigCraft que a LegatIA ainda não mapeou fica com o valor deles (`..t` em `legatia.rs`),
  em vez de quebrar o build: vale olhar `Tokens` em `theme.rs` a cada atualização.
- Não suba arquivos de fonte com licença que proíba redistribuir: o repositório e as releases são públicos.
