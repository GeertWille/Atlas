// Shared by the browse fixtures: says on the console what input reached the
// page and whether a person's devices would have caused it.
(function () {
  var seen = {};
  ['pointermove', 'wheel', 'touchstart', 'mousedown', 'keydown', 'keypress', 'input'].forEach(function (type) {
    addEventListener(
      type,
      function (event) {
        if (seen[type]) return;
        seen[type] = true;
        console.log('FIRST ' + type + ' trusted=' + event.isTrusted);
      },
      true
    );
  });
  addEventListener(
    'click',
    function (event) {
      var link = event.target.closest ? event.target.closest('a') : null;
      console.log('CLICK trusted=' + event.isTrusted + ' on=' + (link ? link.id : event.target.tagName));
    },
    true
  );
  console.log('BINDING visible=' + ('__adscan_emit' in window));
})();
