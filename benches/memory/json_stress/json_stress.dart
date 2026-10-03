// json_stress: ~1.5 MB document of 20000 flat objects,
// 5 rounds of JSON parse + encode, checksum = byte sum of output.
import 'dart:convert';

void main() {
  const piece = '{"id": 7, "name": "name-7", "tags": ["a", "b", "c"], "score": 1.5, "ok": true}';
  final chunk = List.filled(500, piece).join(',');
  final doc = '[${List.filled(40, chunk).join(',')}]';
  print('RESULT doclen ${doc.length}');

  var charsum = 0;
  for (var r = 0; r < 5; r++) {
    final back = jsonEncode(jsonDecode(doc));
    for (var i = 0; i < back.length; i++) {
      charsum += back.codeUnitAt(i);
    }
  }
  print('RESULT checksum $charsum');
}
