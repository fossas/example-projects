module Gnarly
  ( gnarlify
  ) where

import Awesome.Prelude

import Data.Aeson (eitherDecode)
import Data.ByteString.Lazy as BSL (ByteString)

gnarlify :: BSL.ByteString -> Either String Integer
gnarlify = eitherDecode @Integer
