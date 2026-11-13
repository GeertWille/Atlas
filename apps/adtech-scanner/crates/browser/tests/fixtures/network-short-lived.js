// Fetches `asks`, `times` times in a row, and resolves once the last answer
// was read to its end; with `unread`, as soon as its headers are in.
function fetchAsAsked(task) {
  var left = Number(task.get('times'));
  function once() {
    left -= 1;
    return fetch(task.get('asks'))
      .then(function (answer) { return task.get('unread') ? null : answer.text(); })
      .then(function () { return left > 0 ? once() : null; });
  }
  return once();
}
