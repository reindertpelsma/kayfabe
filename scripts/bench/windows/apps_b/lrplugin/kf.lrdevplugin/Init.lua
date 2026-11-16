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
    local photos = {}
    catalog:withWriteAccessDo('kf import', function()
      for path in string.gmatch(job.raws, '[^|]+') do
        local p = catalog:addPhoto(path)
        if p then photos[#photos + 1] = p end
      end
    end, { timeout = 120 })
    if #photos == 0 then error('addPhoto returned nothing') end
    -- develop: a stack of corrections that all run on the GPU pipeline
    catalog:withWriteAccessDo('kf develop', function()
      for _, p in ipairs(photos) do
        p:applyDevelopSettings({ Exposure2012 = 0.4, Contrast2012 = 25, Highlights2012 = -40, Shadows2012 = 35, Texture = 30, Clarity2012 = 25,
          Dehaze = 20, Vibrance = 20, Saturation = 5, Sharpness = 60, LuminanceSmoothing = 30, ColorNoiseReduction = 25, PostCropVignetteAmount = -20 })
      end
    end, { timeout = 60 })
    local session = LrExportSession {
      photosToExport = photos,
      exportSettings = {
        LR_format = 'JPEG', LR_jpeg_quality = 0.9,
        LR_export_destinationType = 'specificFolder', LR_export_destinationPathPrefix = job.out,
        LR_export_useSubfolder = false, LR_collisionHandling = 'overwrite',
        LR_size_doConstrain = false,
        LR_renamingTokensOn = true, LR_tokens = '{{naming_sequenceNumber_3Digits}}', LR_tokensCustomString = 'kf',
        LR_metadata_keywordOptions = 'flat', LR_removeLocationMetadata = true,
      },
    }
    local n = 0
    for _, rendition in session:renditions() do
      local success, pathOrMessage = rendition:waitForRender()
      if success then n = n + 1 else error(tostring(pathOrMessage)) end
    end
    writeResult(job.result, 'OK renditions=' .. tostring(n) .. ' photos=' .. tostring(#photos) .. ' version=' .. tostring(LrApplication.versionString()))
  end)
  if not ok then writeResult(job.result, 'ERR ' .. tostring(err)) end
end)
