"""A small reader of the client DBC tables (extract them with `wowtool extract` from DBFilesClient)."""
import struct


class Dbc:
    def __init__(self, path):
        data = open(path, "rb").read()
        magic, self.count, self.fields, self.size, strsize = struct.unpack_from("<4sIIII", data, 0)
        assert magic == b"WDBC", path
        self.data, self.strings = data, data[20 + self.count * self.size:]
        self.base = 20

    def u32(self, row, field):
        return struct.unpack_from("<I", self.data, self.base + row * self.size + field * 4)[0]

    def i32(self, row, field):
        return struct.unpack_from("<i", self.data, self.base + row * self.size + field * 4)[0]

    def f32(self, row, field):
        return struct.unpack_from("<f", self.data, self.base + row * self.size + field * 4)[0]

    def string(self, row, field):
        off = self.u32(row, field)
        end = self.strings.index(b"\0", off)
        return self.strings[off:end].decode("utf8", "replace")

    def __len__(self):
        return self.count
