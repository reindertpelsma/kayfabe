-- Runs once when Lightroom Classic loads the plugin: reads the job file, imports the raw photo into the active catalog and exports a
-- 1920-wide JPEG through the develop pipeline. Writes a one-line result file. No network, no account.
local LrTasks = import 'LrTasks'
local LrApplication = import 'LrApplication'
local LrExportSession = import 'LrExportSession'
local LrPathUtils = import 'LrPathUtils'

local function readJob()
  local f = io.open('C:\\kfhw\\lr_job.txt', 'r')
  if not f then return nil end
  local job = {}
  for line in f:lines() do
    local k, v = line:match('^(%w+)=(.*)$')
    if k then job[k] = v end
  end
  f:close()
  return job
end

local function writeResult(path, text)
  local f = io.open(path, 'w')
  if f then f:write(text) f:close() end
end

LrTasks.startAsyncTask(function()
  local job = readJob()
  if not job then return end
  local ok, err = pcall(function()
    local catalog = LrApplication.activeCatalog()
    local photo
    catalog:withWriteAccessDo('kf import', function()
      photo = catalog:addPhoto(job.raw)
    end, { timeout = 60 })
    if not photo then error('addPhoto returned nothing') end
    local session = LrExportSession {
      photosToExport = { photo },
      exportSettings = {
        LR_format = 'JPEG', LR_jpeg_quality = 0.9,
        LR_export_destinationType = 'specificFolder', LR_export_destinationPathPrefix = job.out,
        LR_export_useSubfolder = false, LR_collisionHandling = 'overwrite',
        LR_size_doConstrain = true, LR_size_resizeType = 'wh', LR_size_maxWidth = 1920, LR_size_maxHeight = 1920, LR_size_units = 'pixels',
        LR_renamingTokensOn = true, LR_tokens = 'kfexport', LR_tokensCustomString = 'kfexport',
        LR_metadata_keywordOptions = 'flat', LR_removeLocationMetadata = true,
      },
    }
    local n = 0
    for _, rendition in session:renditions() do
      local success, pathOrMessage = rendition:waitForRender()
      if success then n = n + 1 else error(tostring(pathOrMessage)) end
    end
    writeResult(job.result, 'OK renditions=' .. tostring(n) .. ' version=' .. tostring(LrApplication.versionString()))
  end)
  if not ok then writeResult(job.result, 'ERR ' .. tostring(err)) end
end)
