# oedb-rs

> English version below — [English](#english)

Instance **[OpenEventDatabase](https://github.com/openeventdatabase/backend)** statique
pour le **Vaucluse (FR-84)**, générée en **Rust** et servie par **GitHub Pages**.

Consommée par la couche « Événements » de
[dataroads-FR84](https://github.com/thepriben/dataroads-FR84).

## Sources de données

| Source | Catégories OEDB (`what`) | Rythme |
|---|---|---|
| [Bison Futé / DIR](http://tipi.bison-fute.gouv.fr/) (flux DATEX II, filtré Vaucluse) | `traffic.accident`, `traffic.roadwork`, `traffic.jam`, `traffic.hazard`, `weather.road`, `traffic.info` | build toutes les 3 h |
| Curation manuelle (`events/curated/*.yaml`) — ex. [Jeudis d'Orange](https://www.ville-orange.fr/article2431.html), matchs de rugby [Fédérale 2](https://www.rugbyrama.fr/resultats/rugby/federale-2/calendrier) (Avignon Le Pontet, Cavaillon) | `culture.market.night`, `sport.rugby.match`, … | à chaque build |
| Issues GitHub étiquetées `event` + `approved` (formulaire) | toutes | à chaque build |

Les événements expirés (`stop` passé depuis plus de 24 h) sont purgés à chaque build.

## Liaison Wikidata (optionnelle)

Chaque événement peut porter jusqu'à trois QID Wikidata, émis dans les propriétés
GeoJSON quand ils sont renseignés :

| Propriété | Sens | Exemple |
|---|---|---|
| `type_wikidata` | Le *type* d'événement | [`Q1962840`](https://www.wikidata.org/wiki/Q1962840) (marché nocturne) |
| `place_wikidata` | Le lieu | [`Q187796`](https://www.wikidata.org/wiki/Q187796) (Orange) |
| `wikidata` | L'événement ou la série elle-même | un festival précis, si l'entité existe |

Tout fonctionne **sans** Wikidata : les champs sont optionnels et la taxonomie OEDB
`what` reste le classifieur obligatoire. Le formulaire du site propose une
autocomplétion (`wbsearchentities`) pour remplir ces champs sans connaître les QID.

## Étalement des points partagés

Les événements aux coordonnées strictement identiques (ex. les quatre soirées des
Jeudis d'Orange, toutes place du centre-ville) sont répartis à chaque build sur un
petit cercle déterministe (~25 m, tri par id) : chaque occurrence a sa propre
position, stable d'un build à l'autre, et la carte de contrôle les regroupe en
grappes au dézoom.

## API (lecture, compatible OEDB)

Base : `https://thepriben.github.io/oedb-rs`

| Route | Contenu |
|---|---|
| `GET /api/event.json` | FeatureCollection GeoJSON des événements actifs. Chaque Feature porte les propriétés OEDB : `id`, `what` (taxonomie pointée), `type` (`scheduled`/`unscheduled`), `start`/`stop` (ISO 8601), `label`, `source`, `createdate`, `lastupdate` |
| `GET /api/event/{id}.json` | Un événement individuel |
| `GET /api/stats.json` | Compteurs par catégorie et par type (équivalent `/stats` OEDB) |
| `GET /api/vaucluse.geojson` | Alias GeoJSON de la collection |

Les requêtes par secteur (`bbox`, `what`, `when`) ne peuvent pas s'exécuter côté
serveur en statique : le client [`oedb-client.js`](site/oedb-client.js) télécharge la
collection (petite : Vaucluse uniquement) et applique les mêmes filtres que l'API OEDB.

```js
const events = await OedbClient.fetchEvents('https://thepriben.github.io/oedb-rs', {
    bbox: [4.6, 43.8, 5.2, 44.3],
    what: 'traffic',
    when: new Date()
});
```

## Écriture : différence assumée avec l'instance dynamique

L'API OEDB de référence expose `POST /event`. Ici, le chemin d'écriture est :

1. le [formulaire](https://thepriben.github.io/oedb-rs/) (ou l'[issue form](../../issues/new?template=event.yml))
   crée une **issue GitHub** structurée, étiquetée `event` ;
2. un mainteneur pose le label **`approved`** ;
3. le workflow de build parse l'issue, valide l'événement (taxonomie, dates,
   position dans l'emprise FR-84), le persiste dans `events/from-issues/` et
   redéploie l'API.

## Développement

```bash
cargo test                                   # schéma, DATEX II, curation, issues
cargo run --release -- --out dist            # build complet (réseau)
OEDB_OFFLINE=1 cargo run -- --out dist       # build hors-ligne (curation seule)
```

Variables : `OEDB_DATEX_URL` (flux DATEX II), `GITHUB_TOKEN` / `GITHUB_REPOSITORY`
(lecture des issues), `OEDB_OFFLINE=1` (aucun appel réseau).

## Architecture

- `src/model.rs` — types du schéma OEDB + validation (taxonomie, dates, bbox FR-84)
- `src/ingest/datex.rs` — flux DATEX II Bison Futé → `traffic.*`
- `src/ingest/curated.rs` — curation YAML (`events/curated/`, `events/from-issues/`)
- `src/ingest/issues.rs` — issues GitHub `event`+`approved` → événements persistés
- `src/emit.rs` — arborescence statique `/api` (collection, fichiers par id, stats)
- `site/` — page de présentation, carte de contrôle, formulaire, `oedb-client.js`
- `.github/workflows/build.yml` — cron 3 h + label `approved` → build + déploiement Pages

---

## English

Static **[OpenEventDatabase](https://github.com/openeventdatabase/backend)**-compatible
instance for **Vaucluse (FR-84)**, generated in **Rust** and served by **GitHub Pages**.
Consumed by the “Events” layer of [dataroads-FR84](https://github.com/thepriben/dataroads-FR84).

- **Read API**: `GET /api/event.json` (GeoJSON FeatureCollection with OEDB properties),
  `GET /api/event/{id}.json`, `GET /api/stats.json`. Sector queries (`bbox`, `what`,
  `when`) are provided client-side by [`oedb-client.js`](site/oedb-client.js).
- **Write path** (assumed difference with the dynamic OEDB `POST /event`): a
  [form](https://thepriben.github.io/oedb-rs/) pre-fills a GitHub issue; once a
  maintainer adds the `approved` label, the build workflow validates the event,
  persists it under `events/from-issues/` and redeploys.
- **Data sources**: Bison Futé DATEX II feed filtered on Vaucluse (`traffic.*`),
  manually curated YAML events (e.g. the
  [Jeudis d'Orange](https://www.ville-orange.fr/article2431.html) night markets,
  `culture.market.night`, and Fédérale 2 rugby matches, `sport.rugby.match`), and
  approved GitHub issues. Expired events are purged at every build (every 3 hours).
- **Optional Wikidata linkage**: events may carry `wikidata`, `type_wikidata` and
  `place_wikidata` QIDs, emitted as GeoJSON properties; the submission form offers
  Wikidata autocomplete. Everything works without them.
- **Shared-point spreading**: events with identical coordinates are deterministically
  spread on a ~25 m circle so each occurrence gets its own stable position.

## Licence / License

[MIT](LICENSE)
