/**
 * oedb-client.js — tiny read client for the static OEDB instance.
 *
 * The instance is fully static (GitHub Pages), so sector queries (`bbox`,
 * `what`, `when`) cannot run server-side. This client downloads the whole
 * collection (small: Vaucluse only) and applies the same filters as the
 * OpenEventDatabase `GET /event` API.
 *
 * Usage:
 *   const events = await OedbClient.fetchEvents('https://thepriben.github.io/oedb-rs', {
 *       bbox: [4.6, 43.8, 5.2, 44.3],        // [lonMin, latMin, lonMax, latMax]
 *       what: 'traffic',                      // taxonomy prefix
 *       when: new Date()                      // active at this instant
 *   });
 */
(function (global) {
    'use strict';

    async function fetchCollection(base) {
        const url = `${String(base).replace(/\/+$/, '')}/api/event.json`;
        const response = await fetch(url, { cache: 'no-cache', credentials: 'omit' });
        if (!response.ok) {
            throw new Error(`OEDB: HTTP ${response.status}`);
        }
        return response.json();
    }

    function matchesBbox(feature, bbox) {
        if (!bbox) return true;
        const coords = feature.geometry && feature.geometry.coordinates;
        if (!coords) return false;
        const [lon, lat] = coords;
        return lon >= bbox[0] && lat >= bbox[1] && lon <= bbox[2] && lat <= bbox[3];
    }

    function matchesWhat(feature, what) {
        if (!what) return true;
        const value = feature.properties && feature.properties.what;
        return typeof value === 'string' && (value === what || value.startsWith(`${what}.`));
    }

    function matchesWhen(feature, when) {
        if (!when) return true;
        const at = when instanceof Date ? when : new Date(when);
        const props = feature.properties || {};
        if (props.start && new Date(props.start) > at) return false;
        if (props.stop && new Date(props.stop) < at) return false;
        return true;
    }

    async function fetchEvents(base, filters = {}) {
        const collection = await fetchCollection(base);
        const features = (collection.features || []).filter((feature) =>
            matchesBbox(feature, filters.bbox)
            && matchesWhat(feature, filters.what)
            && matchesWhen(feature, filters.when));
        return { ...collection, features };
    }

    global.OedbClient = Object.freeze({ fetchCollection, fetchEvents });
})(typeof window !== 'undefined' ? window : globalThis);
