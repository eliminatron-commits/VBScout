// Writes the containers of the Office test files with independent libraries: compound files
// with Apache POI (POIFS), Access databases with Jackcess (from the empty databases Access made
// that ship with Jackcess). The VBA project streams come from make-binaries.py; this tool only
// places them. It also prints what Apache POI's VBA reader finds, as a cross-check.
//
//   java -cp '<jars>/*' OfficeFixtures.java cfb <out> <spec>
//   java -cp '<jars>/*' OfficeFixtures.java access <V2000|V2003|V2010> <out> <spec>
//   java -cp '<jars>/*' OfficeFixtures.java macros <file>
//   java -cp '<jars>/*' OfficeFixtures.java objects <database> <folder>
//
// A spec has one instruction per line, fields separated by a tab:
//   base <file>                  start from a copy of this compound file (cfb only)
//   remove <path>                remove a storage or stream with everything below it
//   storage <path>               create a storage (and its parents)
//   stream <path> <file>         add or replace a stream (storages are created as needed)
// Paths use '/' (compound file) or are relative to MSysAccessStorage_ROOT (Access).

import com.healthmarketscience.jackcess.Database;
import com.healthmarketscience.jackcess.DatabaseBuilder;
import com.healthmarketscience.jackcess.Row;
import com.healthmarketscience.jackcess.Table;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.time.LocalDateTime;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import org.apache.poi.poifs.filesystem.DirectoryEntry;
import org.apache.poi.poifs.filesystem.DocumentEntry;
import org.apache.poi.poifs.filesystem.Entry;
import org.apache.poi.poifs.filesystem.POIFSFileSystem;
import org.apache.poi.poifs.macros.VBAMacroReader;

public class OfficeFixtures {
    public static void main(String[] args) throws Exception {
        switch (args[0]) {
            case "cfb" -> writeCompound(new File(args[1]), spec(new File(args[2])));
            case "access" -> writeAccess(args[1], new File(args[2]), spec(new File(args[3])));
            case "macros" -> printMacros(new File(args[1]));
            case "objects" -> dumpObjects(new File(args[1]), new File(args[2]));
            default -> throw new IllegalArgumentException("unknown command " + args[0]);
        }
    }

    record Instruction(String verb, String path, String file) {}

    static List<Instruction> spec(File file) throws Exception {
        List<Instruction> list = new ArrayList<>();
        for (String line : Files.readAllLines(file.toPath())) {
            if (line.isBlank()) continue;
            String[] parts = line.split("\t");
            list.add(new Instruction(parts[0], parts[1], parts.length > 2 ? parts[2] : null));
        }
        return list;
    }

    // ---- compound files (POIFS) --------------------------------------------------------------

    static void writeCompound(File out, List<Instruction> spec) throws Exception {
        POIFSFileSystem fs = new POIFSFileSystem();
        for (Instruction i : spec) {
            if (i.verb().equals("base")) {
                try (InputStream in = new FileInputStream(i.path())) {
                    fs = new POIFSFileSystem(in);
                }
            }
        }
        apply(fs.getRoot(), spec);
        try (FileOutputStream stream = new FileOutputStream(out)) {
            fs.writeFilesystem(stream);
        }
        fs.close();
    }

    static void apply(DirectoryEntry root, List<Instruction> spec) throws Exception {
        for (Instruction i : spec) {
            switch (i.verb()) {
                case "base" -> {}
                case "remove" -> {
                    Entry entry = find(root, i.path());
                    if (entry != null) delete(entry);
                }
                case "storage" -> {
                    DirectoryEntry dir = root;
                    for (String part : i.path().split("/")) {
                        Entry child = dir.hasEntryCaseInsensitive(part) ? dir.getEntryCaseInsensitive(part) : null;
                        dir = child instanceof DirectoryEntry d ? d : dir.createDirectory(part);
                    }
                }
                case "stream" -> {
                    String[] parts = i.path().split("/");
                    DirectoryEntry dir = root;
                    for (int k = 0; k < parts.length - 1; k++) {
                        Entry child = dir.hasEntryCaseInsensitive(parts[k]) ? dir.getEntryCaseInsensitive(parts[k]) : null;
                        dir = child instanceof DirectoryEntry d ? d : dir.createDirectory(parts[k]);
                    }
                    String name = parts[parts.length - 1];
                    if (dir.hasEntryCaseInsensitive(name)) delete(dir.getEntryCaseInsensitive(name));
                    dir.createDocument(name, new ByteArrayInputStream(Files.readAllBytes(new File(i.file()).toPath())));
                }
                default -> throw new IllegalArgumentException("unknown instruction " + i.verb());
            }
        }
    }

    static Entry find(DirectoryEntry root, String path) throws Exception {
        Entry entry = root;
        for (String part : path.split("/")) {
            if (!(entry instanceof DirectoryEntry dir) || !dir.hasEntryCaseInsensitive(part)) return null;
            entry = dir.getEntryCaseInsensitive(part);
        }
        return entry;
    }

    static void delete(Entry entry) throws Exception {
        if (entry instanceof DirectoryEntry dir) {
            for (Entry child : list(dir)) delete(child);
        }
        entry.delete();
    }

    static List<Entry> list(DirectoryEntry dir) {
        List<Entry> entries = new ArrayList<>();
        dir.getEntries().forEachRemaining(entries::add);
        return entries;
    }

    // ---- Access databases (Jackcess) ---------------------------------------------------------

    static void writeAccess(String format, File out, List<Instruction> spec) throws Exception {
        out.delete();
        Database.FileFormat fileFormat = Database.FileFormat.valueOf(format);
        try (Database db = new DatabaseBuilder(out).setFileFormat(fileFormat).create()) {
            if (spec.isEmpty()) {
                return; // the empty database as Access made it
            }
            if (fileFormat == Database.FileFormat.V2000) {
                throw new IllegalArgumentException("Access 2000: see make-binaries.py (chunks replaced in place)");
            }
            writeAccessStorage(db, spec);
        }
    }

    /** Access 2002 and later: rows of MSysAccessStorage form a tree of storages (1) and streams (2). */
    static void writeAccessStorage(Database db, List<Instruction> spec) throws Exception {
        Table table = db.getSystemTable("MSysAccessStorage");
        for (Instruction i : spec) {
            if (!i.verb().equals("stream")) throw new IllegalArgumentException("access: only stream instructions");
            Map<Integer, Row> rows = new HashMap<>();
            for (Row row : table) rows.put((Integer) row.get("Id"), row);
            Integer parent = rows.values().stream()
                .filter(r -> "MSysAccessStorage_ROOT".equals(r.get("Name"))).map(r -> (Integer) r.get("Id")).findFirst().orElseThrow();
            String[] parts = i.path().split("/");
            for (int k = 0; k < parts.length; k++) {
                boolean last = k == parts.length - 1;
                final Integer p = parent;
                final String name = parts[k];
                Row existing = rows.values().stream()
                    .filter(r -> p.equals(r.get("ParentId")) && name.equalsIgnoreCase((String) r.get("Name")) && !p.equals(r.get("Id")))
                    .findFirst().orElse(null);
                byte[] data = last ? Files.readAllBytes(new File(i.file()).toPath()) : null;
                if (existing != null) {
                    if (last) {
                        existing.put("Lv", data);
                        existing.put("DateUpdate", LocalDateTime.of(2026, 9, 26, 12, 0));
                        table.updateRow(existing);
                    }
                    parent = (Integer) existing.get("Id");
                } else {
                    Map<String, Object> row = new LinkedHashMap<>();
                    row.put("Name", name);
                    row.put("ParentId", p);
                    row.put("Type", last ? 2 : 1);
                    row.put("DateCreate", LocalDateTime.of(2026, 9, 26, 12, 0));
                    row.put("DateUpdate", LocalDateTime.of(2026, 9, 26, 12, 0));
                    row.put("Lv", data);
                    Map<String, Object> added = table.addRowFromMap(row);
                    parent = (Integer) added.get("Id");
                    rows.clear();
                    for (Row r : table) rows.put((Integer) r.get("Id"), r);
                }
            }
        }
    }

    /**
     * Access 2000: writes every row of MSysAccessObjects to row-<ID>.bin and the compound file
     * they hold to compound.bin. (Jackcess reads these rows but cannot write the column type, so
     * make-binaries.py replaces the chunks in place.)
     */
    static void dumpObjects(File database, File folder) throws Exception {
        folder.mkdirs();
        try (Database db = DatabaseBuilder.open(database)) {
            Files.write(new File(folder, "compound.bin").toPath(), objectsCompound(db, folder));
        }
    }

    static byte[] objectsCompound(Database db, File folder) throws Exception {
        TreeMap<Integer, byte[]> rows = new TreeMap<>();
        for (Row row : db.getSystemTable("MSysAccessObjects")) rows.put((Integer) row.get("ID"), (byte[]) row.get("Data"));
        ByteArrayOutputStream joined = new ByteArrayOutputStream();
        for (Map.Entry<Integer, byte[]> e : rows.entrySet()) {
            if (folder != null) Files.write(new File(folder, "row-" + e.getKey() + ".bin").toPath(), e.getValue());
            if (e.getKey() > 0) joined.write(e.getValue());
        }
        // The header names the length; the allocation table may cover whole sectors beyond it.
        int length = ByteBuffer.wrap(rows.get(0)).order(ByteOrder.LITTLE_ENDIAN).getInt(4);
        byte[] bytes = joined.toByteArray();
        return Arrays.copyOf(bytes, Math.max(length, bytes.length / 512 * 512));
    }

    // ---- cross-check -------------------------------------------------------------------------

    static void printMacros(File file) throws Exception {
        try (VBAMacroReader reader = new VBAMacroReader(file)) {
            for (Map.Entry<String, String> e : new TreeMap<>(reader.readMacros()).entrySet()) {
                String code = e.getValue().replace("\r\n", "\n");
                System.out.println(e.getKey() + "\t" + code.length() + "\t" + Integer.toHexString(code.hashCode()));
            }
        }
    }
}
