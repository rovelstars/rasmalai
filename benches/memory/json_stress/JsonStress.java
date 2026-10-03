// json_stress: ~1.5 MB document of 20000 flat objects,
// 5 rounds of parse + compact reserialize, checksum = byte sum of output.
import java.util.ArrayList;
import java.util.List;

public class JsonStress {
    interface Jval {
        void ser(StringBuilder o);
    }

    record Jint(long v) implements Jval {
        public void ser(StringBuilder o) { o.append(v); }
    }
    record Jnum(double v) implements Jval {
        public void ser(StringBuilder o) {
            if (v == Math.rint(v) && Math.abs(v) < 1e15) o.append((long) v).append(".0");
            else o.append(v);
        }
    }
    record Jstr(String v) implements Jval {
        public void ser(StringBuilder o) { o.append('"').append(v).append('"'); }
    }
    record Jbool(boolean v) implements Jval {
        public void ser(StringBuilder o) { o.append(v ? "true" : "false"); }
    }
    record Jnull() implements Jval {
        public void ser(StringBuilder o) { o.append("null"); }
    }
    record Jarr(List<Jval> items) implements Jval {
        public void ser(StringBuilder o) {
            o.append('[');
            for (int i = 0; i < items.size(); i++) {
                if (i > 0) o.append(',');
                items.get(i).ser(o);
            }
            o.append(']');
        }
    }
    record Jobj(List<String> keys, List<Jval> vals) implements Jval {
        public void ser(StringBuilder o) {
            o.append('{');
            for (int i = 0; i < keys.size(); i++) {
                if (i > 0) o.append(',');
                o.append('"').append(keys.get(i)).append("\":");
                vals.get(i).ser(o);
            }
            o.append('}');
        }
    }

    static class Parser {
        final String s;
        int pos;
        Parser(String s) { this.s = s; }

        void ws() {
            while (pos < s.length()) {
                char c = s.charAt(pos);
                if (c == ' ' || c == '\t' || c == '\n' || c == '\r') pos++;
                else break;
            }
        }

        String parseStr() {
            pos++; // opening quote
            int start = pos;
            while (s.charAt(pos) != '"') {
                if (s.charAt(pos) == '\\') pos++;
                pos++;
            }
            String o = s.substring(start, pos);
            pos++; // closing quote
            return o;
        }

        Jval parseVal() {
            ws();
            char c = s.charAt(pos);
            if (c == '{') {
                pos++;
                List<String> keys = new ArrayList<>();
                List<Jval> vals = new ArrayList<>();
                ws();
                if (s.charAt(pos) != '}') {
                    for (;;) {
                        ws();
                        keys.add(parseStr());
                        ws();
                        pos++; // colon
                        vals.add(parseVal());
                        ws();
                        if (s.charAt(pos) == ',') { pos++; continue; }
                        break;
                    }
                }
                ws();
                pos++; // closing brace
                return new Jobj(keys, vals);
            }
            if (c == '[') {
                pos++;
                List<Jval> items = new ArrayList<>();
                ws();
                if (s.charAt(pos) != ']') {
                    for (;;) {
                        items.add(parseVal());
                        ws();
                        if (s.charAt(pos) == ',') { pos++; continue; }
                        break;
                    }
                }
                ws();
                pos++; // closing bracket
                return new Jarr(items);
            }
            if (c == '"') return new Jstr(parseStr());
            if (c == 't') { pos += 4; return new Jbool(true); }
            if (c == 'f') { pos += 5; return new Jbool(false); }
            if (c == 'n') { pos += 4; return new Jnull(); }
            int start = pos;
            boolean isFloat = false;
            if (s.charAt(pos) == '-') pos++;
            while (Character.isDigit(s.charAt(pos))) pos++;
            if (s.charAt(pos) == '.') {
                isFloat = true;
                pos++;
                while (Character.isDigit(s.charAt(pos))) pos++;
            }
            if (s.charAt(pos) == 'e' || s.charAt(pos) == 'E') {
                isFloat = true;
                pos++;
                if (s.charAt(pos) == '+' || s.charAt(pos) == '-') pos++;
                while (Character.isDigit(s.charAt(pos))) pos++;
            }
            String text = s.substring(start, pos);
            if (isFloat) return new Jnum(Double.parseDouble(text));
            return new Jint(Long.parseLong(text));
        }
    }

    public static void main(String[] args) {
        String piece = "{\"id\": 7, \"name\": \"name-7\", \"tags\": [\"a\", \"b\", \"c\"], \"score\": 1.5, \"ok\": true}";
        StringBuilder cb = new StringBuilder();
        for (int i = 0; i < 500; i++) cb.append(i > 0 ? "," : "").append(piece);
        StringBuilder db = new StringBuilder("[");
        for (int i = 0; i < 40; i++) {
            if (i > 0) db.append(',');
            db.append(cb);
        }
        db.append(']');
        String doc = db.toString();
        System.out.println("RESULT doclen " + doc.length());

        long charsum = 0;
        for (int r = 0; r < 5; r++) {
            Jval v = new Parser(doc).parseVal();
            StringBuilder o = new StringBuilder();
            v.ser(o);
            for (int i = 0; i < o.length(); i++) charsum += o.charAt(i);
        }
        System.out.println("RESULT checksum " + charsum);
    }
}
