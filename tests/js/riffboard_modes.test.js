'use strict';
// Pure helpers behind the song map (static/riffboard/modes.js).
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');

const { sectionKind, sectionLabel, layoutSections, fmt } =
    require(path.join(__dirname, '..', '..', 'static', 'riffboard', 'modes.js'));

test('section names map to colour kinds', () => {
    assert.equal(sectionKind('Verse 2'), 'verse');
    assert.equal(sectionKind('prechorus'), 'pre');
    assert.equal(sectionKind('Pre-Chorus'), 'pre');
    assert.equal(sectionKind('CHORUS'), 'chorus');
    assert.equal(sectionKind('noguitar'), 'rest');
    assert.equal(sectionKind('Section 4'), 'other');
    assert.equal(sectionKind(null), 'other');
});

test('section labels are tidied', () => {
    assert.equal(sectionLabel('verse2'), 'Verse 2');
    assert.equal(sectionLabel('Section 3'), 'Section 3');
    assert.equal(sectionLabel('pre_chorus'), 'Pre chorus');
    assert.equal(sectionLabel(''), 'Section');
});

test('layout covers the song, sorted, with a lead-in block', () => {
    const blocks = layoutSections([
        { name: 'chorus', time: 60 },
        { name: 'verse', time: 10 },
        { name: 'outro', time: 90 },
    ], 100);
    assert.deepEqual(blocks.map((b) => b.kind), ['other', 'verse', 'chorus', 'outro']);
    assert.equal(blocks[0].label, 'Start');
    assert.equal(blocks[0].start, 0);
    assert.equal(blocks.at(-1).end, 100);
    const total = blocks.reduce((s, b) => s + b.width, 0);
    assert.ok(Math.abs(total - 1) < 1e-9);
});

test('layout tolerates bad input', () => {
    assert.deepEqual(layoutSections(null, 100), []);
    assert.deepEqual(layoutSections([{ name: 'a', time: 1 }], 0), []);
    const blocks = layoutSections([
        { name: 'a', time: 0 }, { name: 'dup', time: 0 }, { name: 'late', time: 500 },
        { name: 'bad', time: 'x' }, null,
    ], 100);
    // Zero-length duplicates and past-the-end sections collapse away.
    assert.ok(blocks.every((b) => b.width > 0));
    assert.equal(blocks.at(-1).end, 100);
});

test('time format', () => {
    assert.equal(fmt(0), '0:00');
    assert.equal(fmt(65.9), '1:05');
    assert.equal(fmt(-3), '0:00');
});
