// Minimal ZIP (store) writer for bundling files into a single .zip
// Comments are in English per project guideline
(function () {
    function strToU8(s) { return new TextEncoder().encode(s); }

    // Precompute CRC32 table
    var crcTable = (function () {
        var c, table = new Uint32Array(256);
        for (var n = 0; n < 256; n++) {
            c = n;
            for (var k = 0; k < 8; k++) {
                c = (c & 1) ? (0xEDB88320 ^ (c >>> 1)) : (c >>> 1);
            }
            table[n] = c >>> 0;
        }
        return table;
    })();

    function crc32(u8) {
        var c = 0 ^ (-1);
        for (var i = 0; i < u8.length; i++) {
            c = (c >>> 8) ^ crcTable[(c ^ u8[i]) & 0xFF];
        }
        return (c ^ (-1)) >>> 0;
    }

    function le16(n) { return new Uint8Array([n & 0xFF, (n >> 8) & 0xFF]); }
    function le32(n) { return new Uint8Array([n & 0xFF, (n >> 8) & 0xFF, (n >> 16) & 0xFF, (n >> 24) & 0xFF]); }

    function concat(arrays) {
        var len = 0;
        for (var i = 0; i < arrays.length; i++) len += arrays[i].length;
        var out = new Uint8Array(len);
        var off = 0;
        for (var i = 0; i < arrays.length; i++) { out.set(arrays[i], off); off += arrays[i].length; }
        return out;
    }

    function createZipBlob(files) {
        var localParts = [];
        var centralParts = [];
        var offset = 0;

        for (var i = 0; i < files.length; i++) {
            var name = files[i].name || ('file' + i);
            var data = files[i].content;
            if (typeof data === 'string') { data = strToU8(data); }
            else if (data instanceof ArrayBuffer) { data = new Uint8Array(data); }
            else if (!(data instanceof Uint8Array)) { data = strToU8(String(data)); }

            var crc = crc32(data);
            var nameBytes = strToU8(name);

            var lfHeader = concat([
                le32(0x04034b50),    // local file header signature
                le16(20),            // version needed to extract
                le16(0),             // general purpose bit flag
                le16(0),             // compression method (store)
                le16(0),             // last mod time
                le16(0),             // last mod date
                le32(crc),           // CRC-32
                le32(data.length),   // compressed size
                le32(data.length),   // uncompressed size
                le16(nameBytes.length), // file name length
                le16(0)              // extra field length
            ]);
            var local = concat([lfHeader, nameBytes, data]);
            localParts.push(local);

            var cdHeader = concat([
                le32(0x02014b50),    // central directory signature
                le16(20),            // version made by
                le16(20),            // version needed
                le16(0),             // flags
                le16(0),             // method (store)
                le16(0),             // time
                le16(0),             // date
                le32(crc),
                le32(data.length),
                le32(data.length),
                le16(nameBytes.length),
                le16(0),             // extra len
                le16(0),             // comment len
                le16(0),             // disk start
                le16(0),             // int attrs
                le32(0),             // ext attrs
                le32(offset)         // rel offset
            ]);
            var central = concat([cdHeader, nameBytes]);
            centralParts.push(central);
            offset += local.length;
        }

        var centralDir = concat(centralParts);
        var end = concat([
            le32(0x06054b50),    // end of central directory
            le16(0), le16(0),
            le16(files.length),
            le16(files.length),
            le32(centralDir.length),
            le32(offset),
            le16(0)
        ]);
        var zipBytes = concat(localParts.concat([centralDir, end]));
        return new Blob([zipBytes], { type: 'application/zip' });
    }

    window.__zipFiles = createZipBlob;
    window.__zipStrToU8 = strToU8;
})();

