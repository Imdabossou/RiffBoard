"""Small read-only library endpoint: the charts that make up a work.

Upstream kept favourites, tags, saved-for-later, continue-playing and the
keeper-chart choice here, all in the shared server database. On a public
site those belong to each visitor, so RiffBoard keeps them in the browser
(static/riffboard/local-api.js) and only this read stays server-side.
"""

from fastapi import APIRouter

import appstate

router = APIRouter()


@router.get("/api/work/{work_key:path}/charts")
def api_get_work_charts(work_key: str):
    """All charts in a work + which is the auto-picked keeper."""
    return appstate.meta_db.work_charts(work_key)
