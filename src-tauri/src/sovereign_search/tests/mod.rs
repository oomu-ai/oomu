use super::*;

const SEARCH_RESULT_FIXTURE: &str = r#"
<html>
  <body>
    <div class="result">
      <a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com%2Falpha%3Fq%3D1">Alpha Result</a>
      <a class="result__snippet">First public result snippet.</a>
    </div>
    <div class="result">
      <a class="result__a" href="https://example.org/beta#section">Beta Result</a>
      <div class="result__snippet">Second snippet with extra whitespace.</div>
    </div>
    <table>
      <tr>
        <td>
          <a rel="nofollow" href="https://example.net/gamma" class='result-link'>Gamma Result</a>
        </td>
      </tr>
      <tr>
        <td class='result-snippet'>Third lite endpoint snippet.</td>
      </tr>
    </table>
  </body>
</html>
"#;

mod authorization;
mod classification;
mod evidence;
